//! Reaching the server: the Windows Firewall rule, LAN addresses and join links. No UPnP and no
//! public-IP lookup (ADR 0015): the page explains router forwarding instead.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

use serde::{Deserialize, Serialize};

use super::{require_approval, require_windows};
use crate::error::{Error, Result};
use crate::util::hidden_command;

/// The rule name for one protocol, `Sigil-Deadworks-TCP-27067`. No spaces, so it passes through
/// `netsh` and an elevated `cmd /c` without quoting.
pub fn firewall_rule_name(protocol: &str, port: u16) -> String {
    format!("Sigil-Deadworks-{protocol}-{port}")
}

/// Which of the two inbound rules exist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirewallStatus {
    pub tcp: bool,
    pub udp: bool,
}

impl FirewallStatus {
    pub fn open(&self) -> bool {
        self.tcp && self.udp
    }
}

/// Read `netsh advfirewall firewall show rule name=<name>`. netsh exits non-zero and prints "No
/// rules match" (translated on other Windows languages) when the rule is missing; a match prints
/// the rule, name included. Both must agree.
pub fn parse_firewall_show(success: bool, output: &str, name: &str) -> bool {
    success && output.lines().any(|line| line.split_whitespace().any(|word| word == name))
}

fn rule_exists(name: &str) -> Result<bool> {
    let output = hidden_command("netsh")
        .args(["advfirewall", "firewall", "show", "rule"])
        .arg(format!("name={name}"))
        .output()?;
    Ok(parse_firewall_show(output.status.success(), &String::from_utf8_lossy(&output.stdout), name))
}

/// Whether Sigil's TCP and UDP rules for `port` are in place. Reads only, no window.
pub fn firewall_rule_status(port: u16) -> Result<FirewallStatus> {
    require_windows()?;
    Ok(FirewallStatus {
        tcp: rule_exists(&firewall_rule_name("TCP", port))?,
        udp: rule_exists(&firewall_rule_name("UDP", port))?,
    })
}

/// The PowerShell that raises Windows' admin prompt once and runs the `netsh` commands for the
/// missing protocols (`add`) or both (`delete`). The port is a number, so nothing else can ride
/// into the elevated command line. Exit 1223 means the player declined the prompt.
pub fn firewall_script(port: u16, status: FirewallStatus, add: bool) -> String {
    let mut commands = Vec::new();
    for (protocol, present) in [("TCP", status.tcp), ("UDP", status.udp)] {
        let name = firewall_rule_name(protocol, port);
        if add && !present {
            commands.push(format!(
                "netsh advfirewall firewall add rule name={name} dir=in action=allow protocol={protocol} localport={port}"
            ));
        }
        if !add && present {
            commands.push(format!("netsh advfirewall firewall delete rule name={name}"));
        }
    }
    format!(
        "try {{ $p = Start-Process -FilePath cmd.exe -ArgumentList '/c {}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode }} catch {{ exit 1223 }}",
        commands.join(" && ")
    )
}

fn run_elevated(script: &str) -> Result<()> {
    let status = hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .status()?;
    match status.code() {
        Some(0) => Ok(()),
        Some(1223) => Err(Error::msg("Windows did not change the firewall: the admin prompt was declined.")),
        code => Err(Error::msg(format!(
            "Windows did not change the firewall (netsh exit {}).",
            code.map_or_else(|| "unknown".to_string(), |c| c.to_string())
        ))),
    }
}

/// Add inbound TCP and UDP rules for `port`. Shows Windows' admin prompt.
pub fn add_firewall_rule(port: u16, approve: bool) -> Result<FirewallStatus> {
    require_approval(approve, "Opening the firewall port")?;
    require_windows()?;
    let status = firewall_rule_status(port)?;
    if status.open() {
        return Ok(status);
    }
    run_elevated(&firewall_script(port, status, true))?;
    firewall_rule_status(port)
}

/// Remove Sigil's rules for `port`. Shows Windows' admin prompt.
pub fn remove_firewall_rule(port: u16, approve: bool) -> Result<FirewallStatus> {
    require_approval(approve, "Closing the firewall port")?;
    require_windows()?;
    let status = firewall_rule_status(port)?;
    if !status.tcp && !status.udp {
        return Ok(status);
    }
    run_elevated(&firewall_script(port, status, false))?;
    firewall_rule_status(port)
}

/// This PC's private IPv4 addresses. No packet leaves the machine: connecting a UDP socket only
/// asks the routing table which local address would be used for each private range.
pub fn lan_ipv4s() -> Vec<Ipv4Addr> {
    let probes = ["10.255.255.255:9", "172.31.255.255:9", "192.168.255.255:9", "192.0.2.1:9"];
    let mut found = Vec::new();
    for probe in probes {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else { continue };
        if socket.connect(probe).is_err() {
            continue;
        }
        if let Ok(local) = socket.local_addr()
            && let IpAddr::V4(ip) = local.ip()
            && ip.is_private()
            && !found.contains(&ip)
        {
            found.push(ip);
        }
    }
    found
}

/// `steam://connect/<ip>:<port>`: opens Deadlock and joins.
pub fn join_url(ip: IpAddr, port: u16) -> Result<String> {
    let address = match ip {
        IpAddr::V4(v4) => format!("{v4}:{port}"),
        IpAddr::V6(v6) => format!("[{v6}]:{port}"),
    };
    crate::servers::connect_url(&address, None)
}

/// The line players type in the game console: `connect <ip>:<port>`.
pub fn console_connect(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(v4) => format!("connect {v4}:{port}"),
        IpAddr::V6(v6) => format!("connect [{v6}]:{port}"),
    }
}
