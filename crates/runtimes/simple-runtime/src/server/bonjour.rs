use mdns_sd::{ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_imagetranslator._tcp.local.";

pub struct Bonjour {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Bonjour {
    pub fn register(port: u16) -> Result<Self, String> {
        let host = hostname::get()
            .ok()
            .and_then(|s| s.into_string().ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "imagetranslator".into());
        let daemon = ServiceDaemon::new().map_err(|e| format!("mdns daemon: {e}"))?;
        let hostname = format!("{host}.local.");
        let service = ServiceInfo::new(SERVICE_TYPE, &host, &hostname, "", port, None)
            .map_err(|e| format!("mdns service: {e}"))?
            .enable_addr_auto();
        let fullname = service.get_fullname().to_owned();
        daemon
            .register(service)
            .map_err(|e| format!("mdns register: {e}"))?;
        Ok(Self { daemon, fullname })
    }

    pub fn shutdown(self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}
