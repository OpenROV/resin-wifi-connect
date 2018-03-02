use std::thread;
use std::process;
use std::time::Duration;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::error::Error;
use std::net::Ipv4Addr;

use network_manager::{AccessPoint, Connection, ConnectionState, Connectivity, Device, DeviceType,
                      NetworkManager, ServiceState};

use errors::*;
use exit::{exit, trap_exit_signals, ExitResult};
use config::Config;
use dnsmasq::start_dnsmasq;
use server::start_server;

pub enum NetworkCommand {
    Activate,
    Timeout,
    Exit,
    Connect { ssid: String, passphrase: String },
    Disconnect { ssid: String},
}

pub enum NetworkCommandResponse {
    AccessPointsSsids(Vec<String>),
}

struct NetworkCommandHandler {
    manager: NetworkManager,
    device: Device,
    access_points: Vec<AccessPoint>,
    config: Config,
    dnsmasq: process::Child,
    server_tx: Sender<NetworkCommandResponse>,
    network_rx: Receiver<NetworkCommand>,
    activated: bool,
}

impl NetworkCommandHandler {
    fn new(config: &Config, exit_tx: &Sender<ExitResult>) -> Result<Self> {

        // Create a communication channel
        let (network_tx, network_rx) = channel();

        // Manually handle signals in this thread (signal exit of thread upon unix signal)
        Self::spawn_trap_exit_signals(exit_tx, network_tx.clone());

        // Create NM dbus interface
        let manager = NetworkManager::new();
        debug!("NetworkManager connection initialized");

        // Find device for the specified interface, or find the first wifi device
        let device = find_device(&manager, &config.interface)?;

        // Get initial list of access points
        let access_points = get_access_points(&device)?;

        let dnsmasq = start_dnsmasq(config, &device)?;

        let (server_tx, server_rx) = channel();

        Self::spawn_server(config, exit_tx, server_rx, network_tx.clone());

        Self::spawn_activity_timeout(config, network_tx.clone());

        let config = config.clone();
        let activated = false;

        Ok(NetworkCommandHandler {
            manager,
            device,
            access_points,
            config,
            dnsmasq,
            server_tx,
            network_rx,
            activated,
        })
    }

    fn spawn_server(
        config: &Config,
        exit_tx: &Sender<ExitResult>,
        server_rx: Receiver<NetworkCommandResponse>,
        network_tx: Sender<NetworkCommand>,
    ) {
        let gateway = config.gateway;
        let exit_tx_server = exit_tx.clone();
        let ui_directory = config.ui_directory.clone();

        thread::spawn(move || {
            start_server(
                gateway,
                server_rx,
                network_tx,
                exit_tx_server,
                &ui_directory,
            );
        });
    }

    fn spawn_activity_timeout(config: &Config, network_tx: Sender<NetworkCommand>) {
        let activity_timeout = config.activity_timeout;

        if activity_timeout == 0 {
            return;
        }

        thread::spawn(move || {
            thread::sleep(Duration::from_secs(activity_timeout));

            if let Err(err) = network_tx.send(NetworkCommand::Timeout) {
                error!(
                    "Sending NetworkCommand::Timeout failed: {}",
                    err.description()
                );
            }
        });
    }

    fn spawn_trap_exit_signals(exit_tx: &Sender<ExitResult>, network_tx: Sender<NetworkCommand>) {
        let exit_tx_trap = exit_tx.clone();

        // Create thread that monitors for exit signals and translates them to an exit event on the network channel
        thread::spawn(move || {
            if let Err(e) = trap_exit_signals() {
                exit(&exit_tx_trap, e);
                return;
            }

            if let Err(err) = network_tx.send(NetworkCommand::Exit) {
                error!("Sending NetworkCommand::Exit failed: {}", err.description());
            }
        });
    }

    fn run(&mut self, exit_tx: &Sender<ExitResult>) {
        let result = self.run_loop();
        self.stop(exit_tx, result);
    }

    fn run_loop(&mut self) -> ExitResult {
        loop {
            let command = self.receive_network_command()?;

            match command {
                NetworkCommand::Activate => {
                    self.activate()?;
                },
                NetworkCommand::Timeout => {
                    if !self.activated {
                        info!("Timeout reached. Exiting...");
                        return Ok(());
                    }
                },
                NetworkCommand::Exit => {
                    info!("Exiting...");
                    return Ok(());
                },
                NetworkCommand::Connect { ssid, passphrase } => {
                    self.connect(&ssid, &passphrase)?;
                },
                NetworkCommand::Disconnect { ssid } => {
                    self.disconnect(&ssid)?;
                },
            }
        }
    }

    fn receive_network_command(&self) -> Result<NetworkCommand> {
        match self.network_rx.recv() {
            Ok(command) => Ok(command),
            Err(e) => {
                // Sleep for a second, so that other threads may log error info.
                thread::sleep(Duration::from_secs(1));
                Err(e).chain_err(|| ErrorKind::RecvNetworkCommand)
            },
        }
    }

    fn stop(&mut self, exit_tx: &Sender<ExitResult>, result: ExitResult) {
        let _ = self.dnsmasq.kill();

        let _ = exit_tx.send(result);
    }

    fn activate(&mut self) -> ExitResult {
        self.activated = true;

        let access_points_ssids = get_access_points_ssids_owned(&self.access_points);

        self.server_tx
            .send(NetworkCommandResponse::AccessPointsSsids(
                access_points_ssids,
            ))
            .chain_err(|| ErrorKind::SendAccessPointSSIDs)
    }

    fn connect(&mut self, ssid: &str, passphrase: &str) -> Result<bool> {
        delete_connection_if_exists(&self.manager, ssid);

        self.access_points = get_access_points(&self.device)?;

        if let Some(access_point) = find_access_point(&self.access_points, ssid) {
            let wifi_device = self.device.as_wifi_device().unwrap();

            info!("Connecting to access point '{}'...", ssid);

            match wifi_device.connect(access_point, passphrase) {
                Ok((connection, state)) => {
                    if state == ConnectionState::Activated {
                        match wait_for_connectivity(&self.manager, 20) {
                            Ok(has_connectivity) => {
                                if has_connectivity {
                                    info!("Internet connectivity established");
                                } else {
                                    warn!("Cannot establish Internet connectivity");
                                }
                            },
                            Err(err) => error!("Getting Internet connectivity failed: {}", err),
                        }

                        return Ok(true);
                    }

                    if let Err(err) = connection.delete() {
                        error!("Deleting connection object failed: {}", err)
                    }

                    warn!(
                        "Connection to access point not activated '{}': {:?}",
                        ssid, state
                    );
                },
                Err(e) => {
                    warn!("Error connecting to access point '{}': {}", ssid, e);
                },
            }
        }

        self.access_points = get_access_points(&self.device)?;

        Ok(false)
    }

    fn disconnect(&mut self, ssid: &str) -> Result<bool> {
        self.device.disconnect()?;

        Ok(false)
    }
}

pub fn process_network_commands(config: &Config, exit_tx: &Sender<ExitResult>) {
    let mut command_handler = match NetworkCommandHandler::new(config, exit_tx) {
        Ok(command_handler) => command_handler,
        Err(e) => {
            exit(exit_tx, e);
            return;
        },
    };

    command_handler.run(exit_tx);
}

pub fn init_networking() -> Result<()> {
    // Start NetworkManager, if not already running
    start_network_manager_service()?;

    // Delete any existing wifi AP config information
    // TODO: We probably don't want to do this!
    delete_access_point_profiles().chain_err(|| ErrorKind::DeleteAccessPoint)
}

pub fn find_device(manager: &NetworkManager, interface: &Option<String>) -> Result<Device> {

    // Check for wifi device on specified interface
    if let Some(ref interface) = *interface {
        let device = manager
            .get_device_by_interface(interface)
            .chain_err(|| ErrorKind::DeviceByInterface(interface.clone()))?;

        if *device.device_type() == DeviceType::WiFi {
            info!("Targeted WiFi device: {}", interface);
            Ok(device)
        } else {
            bail!(ErrorKind::NotAWiFiDevice(interface.clone()))
        }
    } else {
        // No interface specified, scan for the first detected Wifi interface
        let devices = manager.get_devices()?;

        let index = devices
            .iter()
            .position(|d| *d.device_type() == DeviceType::WiFi);

        if let Some(index) = index {
            info!("WiFi device: {}", devices[index].interface());
            Ok(devices[index].clone())
        } else {
            bail!(ErrorKind::NoWiFiDevice)
        }
    }
}

fn get_access_points(device: &Device) -> Result<Vec<AccessPoint>> {
    get_access_points_impl(device).chain_err(|| ErrorKind::NoAccessPoints)
}

fn get_access_points_impl(device: &Device) -> Result<Vec<AccessPoint>> {
    let retries_allowed = 10;
    let mut retries = 0;

    // After stopping the hotspot we may have to wait a bit for the list
    // of access points to become available
    while retries < retries_allowed {
        let wifi_device = device.as_wifi_device().unwrap();
        let mut access_points = wifi_device.get_access_points()?;

        access_points.retain(|ap| ap.ssid().as_str().is_ok());

        if !access_points.is_empty() {
            info!(
                "Access points: {:?}",
                get_access_points_ssids(&access_points)
            );
            return Ok(access_points);
        }

        retries += 1;
        debug!("No access points found - retry #{}", retries);
        thread::sleep(Duration::from_secs(1));
    }

    warn!("No access points found - giving up...");
    Ok(vec![])
}

fn get_access_points_ssids(access_points: &[AccessPoint]) -> Vec<&str> {
    access_points
        .iter()
        .map(|ap| ap.ssid().as_str().unwrap())
        .collect()
}

fn get_access_points_ssids_owned(access_points: &[AccessPoint]) -> Vec<String> {
    access_points
        .iter()
        .map(|ap| ap.ssid().as_str().unwrap().to_string())
        .collect()
}

fn find_access_point<'a>(access_points: &'a [AccessPoint], ssid: &str) -> Option<&'a AccessPoint> {
    for access_point in access_points.iter() {
        if let Ok(access_point_ssid) = access_point.ssid().as_str() {
            if access_point_ssid == ssid {
                return Some(access_point);
            }
        }
    }

    None
}

fn wait_for_connectivity(manager: &NetworkManager, timeout: u64) -> Result<bool> {
    let mut total_time = 0;

    loop {
        let connectivity = manager.get_connectivity()?;

        if connectivity == Connectivity::Full || connectivity == Connectivity::Limited {
            debug!(
                "Connectivity established: {:?} / {}s elapsed",
                connectivity, total_time
            );

            return Ok(true);
        } else if total_time >= timeout {
            debug!(
                "Timeout reached in waiting for connectivity: {:?} / {}s elapsed",
                connectivity, total_time
            );

            return Ok(false);
        }

        ::std::thread::sleep(::std::time::Duration::from_secs(1));

        total_time += 1;

        debug!(
            "Still waiting for connectivity: {:?} / {}s elapsed",
            connectivity, total_time
        );
    }
}

pub fn start_network_manager_service() -> Result<()> {
    // Get the current state of the network manager service
    let state = NetworkManager::get_service_state().chain_err(|| ErrorKind::NetworkManagerServiceState)?;

    if state != ServiceState::Active {
          // If not active, start the service, with a 15 second timeout value
        let state = NetworkManager::start_service(15).chain_err(|| ErrorKind::StartNetworkManager)?;

        if state != ServiceState::Active {
            // Return error
            bail!(ErrorKind::StartActiveNetworkManager);
        } else {
            info!("NetworkManager service started successfully");
        }
    } else {
        debug!("NetworkManager service already running");
    }

    Ok(())
}

fn delete_access_point_profiles() -> Result<()> {

    // Create reference counted NetworkManager interface
    let manager = NetworkManager::new();

    // Get list of every connection ever configured or stored in NetworkManager
    let connections = manager.get_connections()?;

    for connection in connections {
        // Filter on wifi connection types
        if &connection.settings().kind == "802-11-wireless" && &connection.settings().mode == "ap" {
            debug!(
                "Deleting access point connection profile: {:?}",
                connection.settings().ssid,
            );

            // Delete the connection profile
            connection.delete()?;
        }
    }

    Ok(())
}

fn delete_connection_if_exists(manager: &NetworkManager, ssid: &str) {
    let connections = match manager.get_connections() {
        Ok(connections) => connections,
        Err(e) => {
            error!("Getting existing connections failed: {}", e);
            return;
        },
    };

    for connection in connections {
        if let Ok(connection_ssid) = connection.settings().ssid.as_str() {
            if &connection.settings().kind == "802-11-wireless" && connection_ssid == ssid {
                info!(
                    "Deleting existing WiFi connection: {:?}",
                    connection.settings().ssid,
                );

                if let Err(e) = connection.delete() {
                    error!("Deleting existing WiFi connection failed: {}", e);
                }
            }
        }
    }
}
