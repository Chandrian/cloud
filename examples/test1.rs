use burst::{BurstBuilder, Machine, MachineSetup};
use std::collections::HashMap;

/*
AWS Graviton2 based t4g.micro instances free for up to 750 hours per month
750 free hours per month deducted from their monthly bill through March 2021.
 */
fn main() {
    let mut b = BurstBuilder::default();
    b.add_set(
        "server",
        1,
        MachineSetup::new("t4g.micro", "ami-03192d23e906cf923", |ssh| {
            ssh.cmd("cat /etc/hostname").map(|out| {
                println!("{}", out);
            })
        }),
    );
    b.add_set(
        "client",
        3,
        MachineSetup::new("t4g.micro", "ami-03192d23e906cf923", |ssh| {
            ssh.cmd("date")
                .map(|out| {
                    println!("{}", out);
                })
        }),
    );
    b.run(|vms: HashMap<String, Vec<Machine>>| {
        println!("===> {}", vms["server"][0].private_ip);
        for c in &vms["client"] {
        println!(" -> {}", c.private_ip);
        }
        Ok(())
    });
}
