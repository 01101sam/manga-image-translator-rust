use std::net::IpAddr;
use std::time::Duration;

use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_imagetranslator._tcp.local.";

pub struct Bonjour {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Bonjour {
    pub fn register(port: u16) -> Result<Self, String> {
        let instance = instance_name();
        let hostname = format!("{instance}.local.");
        let addrs = local_ipv4s();
        let daemon = ServiceDaemon::new().map_err(|e| format!("mdns daemon: {e}"))?;
        let monitor = daemon.monitor().map_err(|e| format!("mdns monitor: {e}"))?;
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &hostname,
            addrs.as_str(),
            port,
            &[] as &[(&str, &str)],
        )
        .map_err(|e| format!("mdns service: {e}"))?
        .enable_addr_auto();
        let fullname = service.get_fullname().to_owned();
        daemon
            .register(service)
            .map_err(|e| format!("mdns register: {e}"))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut announced = addrs.is_empty();
        while std::time::Instant::now() < deadline {
            match monitor.recv_timeout(Duration::from_millis(50)) {
                Ok(DaemonEvent::Announce(name, _)) if name.eq_ignore_ascii_case(&fullname) => {
                    announced = true;
                    break;
                }
                Ok(DaemonEvent::Error(e)) => return Err(format!("mdns announce: {e}")),
                Ok(_) | Err(_) => {}
            }
        }
        if !announced {
            eprintln!("bonjour registered {fullname} (no Announce yet) port {port}");
        } else {
            eprintln!("bonjour registered {fullname} port {port}");
        }
        Ok(Self { daemon, fullname })
    }

    pub fn shutdown(self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

fn instance_name() -> String {
    let raw = hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "imagetranslator".into());
    let short = raw
        .split('.')
        .next()
        .unwrap_or("imagetranslator")
        .trim();
    if short.is_empty() {
        "imagetranslator".into()
    } else {
        short.into()
    }
}

fn local_ipv4s() -> String {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return String::new();
    };
    ifaces
        .into_iter()
        .filter_map(|iface| match iface.addr.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() => Some(ip.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(",")
}
