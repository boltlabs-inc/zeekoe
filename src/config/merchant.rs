use std::{net::IpAddr, time::Duration};

pub struct Config {
    address: IpAddr,
    port: u16,
    timeout: Duration,
}
