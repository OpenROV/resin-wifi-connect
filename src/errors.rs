use network_manager;

use network;

error_chain! {
    foreign_links {
        Io(::std::io::Error);
        Recv(::std::sync::mpsc::RecvError);
        SendNetworkCommand(::std::sync::mpsc::SendError<network::NetworkCommand>);
        Nix(::nix::Error);
    }

    links {
        NetworkManager(network_manager::errors::Error, network_manager::errors::ErrorKind);
    }

    errors {
        PingUnsuccessful {
            description( "Pinging public DNS failed" )
        }

        RecvAccessPointSSIDs {
            description("Receiving access point SSIDs failed")
        }

        RecvNetworkCommand {
            description("Receiving network command failed")
        }

        SendNetworkCommandConnect {
            description("Sending NetworkCommand::Connect failed")
        }

        SendNetworkCommandClear {
            description( "Sending NetworkCommand::Clear failed" )
        }

        SendNetworkCommandListAP {
            description( "Sending NetworkCommand::ListAP failed" )
        }

        DeviceByInterface(interface: String) {
            description("Cannot find network device with interface name")
            display("Cannot find network device with interface name '{}'", interface)
        }

        NotAWiFiDevice(interface: String) {
            description("Not a WiFi device")
            display("Not a WiFi device: {}", interface)
        }

        NoWiFiDevice {
            description("Cannot find a WiFi device")
        }

        NoAccessPoints {
            description("Getting access points failed")
        }

        DeleteAccessPoint {
            description("Deleting access point connection profile failed")
        }

        StartHTTPServer(address: String, reason: String) {
            description("Cannot start HTTP server")
            display("Cannot start HTTP server on '{}': {}", address, reason)
        }

        StartActiveNetworkManager {
            description("Starting the NetworkManager service with active state failed")
        }

        StartNetworkManager {
            description("Starting the NetworkManager service failed")
        }

        NetworkManagerServiceState {
            description("Getting the NetworkManager service state failed")
        }

        BlockExitSignals {
            description("Blocking exit signals failed")
        }

        TrapExitSignals {
            description("Trapping exit signals failed")
        }

        RecvAccessPoints {
            description("Receiving access points failed")
        }

        ScanAccessPoints {
            description("Scanning access points failed")
        }
    }
}

pub fn exit_code(e: &Error) -> i32 {
    match *e.kind() {
        
        ErrorKind::RecvAccessPointSSIDs => 4,
        ErrorKind::RecvNetworkCommand => 7,
        ErrorKind::SendNetworkCommandConnect => 9,
        ErrorKind::DeviceByInterface(_) => 10,
        ErrorKind::NotAWiFiDevice(_) => 11,
        ErrorKind::NoWiFiDevice => 12,
        ErrorKind::NoAccessPoints => 13,
        ErrorKind::PingUnsuccessful => 14,
        ErrorKind::SendNetworkCommandClear => 15,
        ErrorKind::DeleteAccessPoint => 16,
        ErrorKind::StartHTTPServer(_, _) => 17,
        ErrorKind::StartActiveNetworkManager => 18,
        ErrorKind::StartNetworkManager => 19,
        ErrorKind::NetworkManagerServiceState => 20,
        ErrorKind::BlockExitSignals => 21,
        ErrorKind::TrapExitSignals => 22,
        ErrorKind::RecvAccessPoints => 24,
        ErrorKind::ScanAccessPoints => 25,
        ErrorKind::SendNetworkCommandListAP => 26,
        _ => 1,
    }
}
