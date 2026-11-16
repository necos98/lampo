use std::net::{Ipv4Addr, TcpListener};

use lampo_domain::ports::{PortAvailability, PortProbe};
use lampo_domain::PortResult;
use lampo_domain::tcp_port::Port;

use crate::win;

/// `WSAEACCES`: the port is in a Windows excluded range (Hyper-V/WSL/Docker reservations) or
/// bound exclusively by another socket.
const WSAEACCES: i32 = 10013;

/// Looks the port up in the TCP listener table (`GetExtendedTcpTable`, IPv4 and IPv6, any local
/// address) to name the owning process; then tries to bind `127.0.0.1:<port>` to catch what the
/// table cannot show (excluded port ranges, bound-but-not-listening sockets). Only loopback is
/// bound, so no Windows Firewall prompt can appear.
#[derive(Debug, Default, Clone, Copy)]
pub struct TcpPortProbe;

impl PortProbe for TcpPortProbe {
    fn check(&self, port: Port) -> PortResult<PortAvailability> {
        if let Ok(Some(pid)) = win::tcp_listener_pid(port.get()) {
            return Ok(PortAvailability::InUse { owner: Some(win::describe_process(pid)) });
        }
        match TcpListener::bind((Ipv4Addr::LOCALHOST, port.get())) {
            Ok(_) => Ok(PortAvailability::Free),
            Err(e) if e.raw_os_error() == Some(WSAEACCES) => Ok(PortAvailability::InUse { owner: Some("Windows (excluded port range or exclusive socket)".into()) }),
            Err(_) => Ok(PortAvailability::InUse { owner: None }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_owner_of_a_busy_port() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = Port::new(listener.local_addr().unwrap().port()).unwrap();
        let me = std::env::current_exe().unwrap();
        let exe = me.file_name().unwrap().to_string_lossy().to_string();
        match TcpPortProbe.check(port).unwrap() {
            PortAvailability::InUse { owner: Some(owner) } => {
                assert_eq!(owner, format!("{exe} (pid {})", std::process::id()));
            }
            other => panic!("expected in use, got {other:?}"),
        }
        drop(listener);
        assert_eq!(TcpPortProbe.check(port).unwrap(), PortAvailability::Free);
    }
}
