use ssh2;
use std::io;
use std::net::{self, TcpStream};

pub struct Session {
    ssh: ssh2::Session,
}

impl Session {
    pub(crate) fn connect<A: net::ToSocketAddrs>(addr: A) -> io::Result<Self> {
        let mut i = 0;

        let tcp = loop {
            match TcpStream::connect(&addr) {
                Ok(s) => break s,
                Err(_) if i <= 3 => {
                    i += 1;
                },
                Err(e) => return Err(e)
            }
        };
        let mut sess = ssh2::Session::new().unwrap();
        sess.set_tcp_stream(tcp);
        sess.handshake()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        sess.userauth_agent("ec2-user")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            
        Ok(Session { ssh: sess })
    }

    pub fn cmd(&mut self, cmd: &str) -> io::Result<String> {
        use std::io::Read;
        // TODO: tell aws to generate a keypair and store it
        let mut channel = self.ssh.channel_session()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        channel
            .exec(cmd)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut s = String::new();
        channel
            .read_to_string(&mut s)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        println!("{}", s);
        channel
            .wait_close()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        println!(
            "{}",
            channel
                .exit_status()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
        );
        // ssh.exec("sudo yum install htop");
        // yum install apache
        Ok(s)
    }
}

use std::ops::{Deref, DerefMut};
impl Deref for Session {
    type Target = ssh2::Session;
    fn deref(&self) -> &Self::Target {
        &self.ssh
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ssh
    }
}
