use std::collections::HashMap;
use std::io;

mod ssh;

pub struct Machine {
    pub ssh: Option<ssh::Session>,
    pub instance_type: String,
    pub private_ip: String,
    pub public_dns: String,
}

pub struct MachineSetup {
    instance_type: String,
    ami: String,
    setup: Box<dyn Fn(&mut ssh::Session) -> io::Result<()>>,
}

impl MachineSetup {
    pub fn new<F>(instance_type: &str, ami: &str, setup: F) -> Self
    where
        F: Fn(&mut ssh::Session) -> io::Result<()> + 'static,
    {
        MachineSetup {
            instance_type: instance_type.to_string(),
            ami: ami.to_string(),
            setup: Box::new(setup),
        }
    }
}

pub struct BurstBuilder {
    descriptors: HashMap<String, (MachineSetup, u32)>,
    max_duration: i64,
}

impl Default for BurstBuilder {
    fn default() -> Self {
        BurstBuilder {
            descriptors: Default::default(),
            max_duration: 60,
        }
    }
}

impl BurstBuilder {
    pub fn add_set(&mut self, name: &str, number: u32, setup: MachineSetup) {
        // TODO: what if name is already in use?
        self.descriptors.insert(name.to_string(), (setup, number));
    }

    pub fn set_max_duration(&mut self, hours: u8) {
        self.max_duration = hours as i64 * 60;
    }

    pub fn run<F>(self, f: F)
    where
        F: FnOnce(HashMap<String, Vec<Machine>>) -> io::Result<()>,
    {
        use rusoto_core::default_tls_client;
        use rusoto_core::{DefaultCredentialsProvider, Region};
        use rusoto_ec2::{
            CancelSpotInstanceRequestsRequest, DescribeInstancesRequest,
            DescribeSpotInstanceRequestsRequest, Ec2, Ec2Client, RequestSpotInstancesRequest,
            RequestSpotLaunchSpecification,
        };
        use rusoto_sts::{StsAssumeRoleSessionCredentialsProvider, StsClient};
        let dispatcher = default_tls_client().unwrap();
        let provider = DefaultCredentialsProvider::new().unwrap();
        let sts = StsClient::new(dispatcher, provider, Region::UsWest2);
        let provider = StsAssumeRoleSessionCredentialsProvider::new(
            sts,
            "arn:aws:iam::593625484736:role/ec2".to_string(),
            "test".to_string(),
            None,
            None,
            None,
            None,
        );
        let ec2 = Ec2Client::new(default_tls_client().unwrap(), provider, Region::UsWest2);
        // 1. issue spot request
        let mut setup_fns = HashMap::new();
        let mut id_to_name = HashMap::new();
        let mut spot_req_ids = Vec::new();
        for (name, (setup, number)) in self.descriptors {
            let mut rsls = RequestSpotLaunchSpecification::default();
            rsls.image_id = Some(setup.ami);
            rsls.instance_type = Some(setup.instance_type);
            setup_fns.insert(name.clone(), setup.setup);
            // TODO
            rsls.security_group_ids = Some(vec!["sg-0cf39477".to_string()]);
            rsls.key_name = Some("burst".to_string());
            // TODO: VPC
            let mut req = RequestSpotInstancesRequest::default();
            req.launch_specification = Some(rsls);
            req.instance_count = Some(i64::from(number));
            req.block_duration_minutes = Some(self.max_duration); //TODO

            let res = ec2.request_spot_instances(&req).unwrap();
            let res = res.spot_instance_requests.unwrap();
            spot_req_ids.extend(
                res.into_iter()
                    .filter_map(|sir| sir.spot_instance_request_id)
                    .map(|sir| {
                        id_to_name.insert(sir.clone(), name.clone());
                        sir
                    }),
            );
        }
        // 2. wait for instances to come up
        let mut req = DescribeSpotInstanceRequestsRequest::default();
        req.spot_instance_request_ids = Some(spot_req_ids);
        let instances: Vec<_>;
        loop {
            let res = ec2.describe_spot_instance_requests(&req);
            if let Err(e) = res {
                let msg = format!("{}", e);
                if msg.contains("The spot instance request ID") && msg.contains("does not exist") {
                    continue;
                }
                    panic!("{}", msg);
            }
            let res = res.unwrap();
            let all_ready = res
                .spot_instance_requests
                .as_ref()
                .unwrap()
                .iter()
                .all(|x| x.state.as_ref().unwrap() == "active");
            if all_ready {
                instances = res
                    .spot_instance_requests
                    .unwrap()
                    .into_iter()
                    .filter_map(|s| {
                        let name = id_to_name.remove(&s.spot_instance_request_id.clone().unwrap());
                        id_to_name.insert(s.instance_id.as_ref().unwrap().clone(), name.unwrap());
                        s.instance_id
                    })
                    .collect();
                break;
            }
        }
        // 3. stop spot requests
        let mut cancel = CancelSpotInstanceRequestsRequest::default();
        cancel.spot_instance_request_ids = req.spot_instance_request_ids.take().unwrap();
        ec2.cancel_spot_instance_requests(&cancel).unwrap();

        // 4. wait until all instances are up
        let mut machines = HashMap::new();
        let mut all_ready = false;
        let mut desc_req = DescribeInstancesRequest::default();
        desc_req.instance_ids = Some(instances);
        while !all_ready {
            all_ready = true;
            machines.clear();
            for reservation in ec2
                .describe_instances(&desc_req)
                .unwrap()
                .reservations
                .unwrap()
            {
                for instance in reservation.instances.unwrap() {
                    match instance {
                        rusoto_ec2::Instance {
                            instance_id: Some(instance_id),
                            instance_type: Some(instance_type),
                            private_ip_address: Some(private_ip),
                            public_dns_name: Some(public_dns),
                            public_ip_address: Some(_),
                            ..
                        } => {
                            let machine = Machine {
                                ssh: None,
                                instance_type,
                                private_ip,
                                public_dns,
                            };
                            let name = id_to_name[&instance_id].clone();
                            machines.entry(name).or_insert_with(Vec::new).push(machine);
                        }
                        _ => {
                            all_ready = false;
                        }
                    }
                }
            }
        }
        //  - once an instance is ready, run setup closure
        for (name, machines) in &mut machines {
            let f = &setup_fns[name];
            // TODO: set up machines in parallel (rayon)
            for machine in machines {
                let mut sess =
                    ssh::Session::connect(&format!("{}:22", machine.public_dns)).unwrap();
                f(&mut sess).unwrap();
                machine.ssh = Some(sess);
            }
        }
        // 5. invoke F with Machine descriptors
        // F: FnOnce(HashMap<String, &mut [Machine]>) -> io::Result<()>,
        f(machines).unwrap();
        // 6. terminate all instances
        let mut termination_req = rusoto_ec2::TerminateInstancesRequest::default();
        termination_req.instance_ids = desc_req.instance_ids.unwrap();
        while let Err(e) = ec2.terminate_instances(&termination_req) {
            let msg = format!("{}", e);
            if msg.contains("Pooled stream disconnected") || msg.contains("broken pipe") {
                continue;
            } else {
                panic!("{}", msg);
            }
        }
    }
}
