use std::sync::mpsc::{Receiver, Sender};
use std::fmt;
use std::error::Error as StdError;

use serde_json;

use path::PathBuf;
use iron::prelude::*;
use iron::{status, typemap, Iron, IronError, IronResult, Request, Response};
use router::Router;
use staticfile::Static;
use mount::Mount;
use persistent::Write;
use params::{FromValue, Params};

use errors::*;
use network::{NetworkCommand, NetworkCommandResponse};
use exit::{exit, ExitResult};

#[derive(Serialize)]
struct AccessPointSerializable {
    ssid: String,
    signal: u32
}

struct RequestSharedState {
    server_rx: Receiver<NetworkCommandResponse>,
    network_tx: Sender<NetworkCommand>,
    exit_tx: Sender<ExitResult>,
}

impl typemap::Key for RequestSharedState {
    type Value = RequestSharedState;
}

#[derive(Debug)]
struct StringError(String);

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl StdError for StringError {
    fn description(&self) -> &str {
        &*self.0
    }
}

macro_rules! get_request_ref {
    ($req:ident, $ty:ty, $err:expr) => (
        match $req.get_ref::<$ty>() {
            Ok(val) => val,
            Err(err) => {
                error!($err);
                return Err(IronError::new(err, status::InternalServerError));
            }
        }
    )
}

macro_rules! get_param {
    ($params:ident, $param:expr, $ty:ty) => (
        match $params.get($param) {
            Some(value) => {
                match <$ty as FromValue>::from_value(value) {
                    Some(converted) => converted,
                    None => {
                        let err = format!("Unexpected type for '{}'", $param);
                        error!("{}", err);
                        return Err(IronError::new(StringError(err), status::InternalServerError));
                    }
                }
            },
            None => {
                let err = format!("'{}' not found in request params: {:?}", $param, $params);
                error!("{}", err);
                return Err(IronError::new(StringError(err), status::InternalServerError));
            }
        }
    )
}

macro_rules! get_request_state {
    ($req:ident) => (
        get_request_ref!(
            $req,
            Write<RequestSharedState>,
            "Getting reference to request shared state failed"
        ).as_ref().lock().unwrap()
    )
}

fn exit_with_error<E>(state: &RequestSharedState, e: E, e_kind: ErrorKind) -> IronResult<Response>
where
    E: ::std::error::Error + Send + 'static,
{
    let description = e_kind.description().into();
    let err = Err::<Response, E>(e).chain_err(|| e_kind);
    exit(&state.exit_tx, err.unwrap_err());
    Err(IronError::new(
        StringError(description),
        status::InternalServerError,
    ))
}

pub fn start_server(
    server_rx: Receiver<NetworkCommandResponse>,
    network_tx: Sender<NetworkCommand>,
    exit_tx: Sender<ExitResult>,
    ui_directory: &PathBuf,
) {
    let exit_tx_clone = exit_tx.clone();
    let request_state = RequestSharedState {
        server_rx: server_rx,
        network_tx: network_tx,
        exit_tx: exit_tx,
    };

    let mut router = Router::new();
    router.get("/", Static::new(ui_directory), "index");
    router.get("/ssids", ssid, "ssids");
    router.get("/connection", ssid, "connection" );
    router.get("/internetAccess", check_internet_connection, "internetAccess" );
    router.post("/connect", connect, "connect");
    router.post("/disconnect", disconnect, "disconnect");
    router.post("/clear", clear_connections, "clear" );
    router.post("/scan", scan, "scan" );

    let mut assets = Mount::new();
    assets.mount("/", router);
    assets.mount("/css", Static::new(&ui_directory.join("css")));
    assets.mount("/img", Static::new(&ui_directory.join("img")));
    assets.mount("/js", Static::new(&ui_directory.join("js")));

    let mut chain = Chain::new(assets);
    chain.link(Write::<RequestSharedState>::both( request_state ));

    let address = String::from( "0.0.0.0:3090" );

    info!("Starting HTTP server on {}", &address);

    if let Err(e) = Iron::new(chain).http(&address) {
        info!("Exiting HTTP server on {}", &address);
        exit(
            &exit_tx_clone,
            ErrorKind::StartHTTPServer(address, e.description().into()).into(),
        );
    }
}

fn scan(req: &mut Request) -> IronResult<Response> {
    let request_state = get_request_state!(req);
    let command = NetworkCommand::Scan;

    if let Err(e) = request_state.network_tx.send(command) {
        exit_with_error(&request_state, e, ErrorKind::ScanAccessPoints)
    } else {
        Ok(Response::with(status::Ok))
    }
}

fn ssid(req: &mut Request) -> IronResult<Response> {
    let request_state = get_request_state!(req);

    if let Err(e) = request_state.network_tx.send(NetworkCommand::ListAP) {
        return exit_with_error(&request_state, e, ErrorKind::SendNetworkCommandListAP);
    }

    let access_points = match request_state.server_rx.recv() {
        Ok(result) => match result {
            NetworkCommandResponse::AccessPointResponse(aps) => aps,
            _ => Vec::new(),
        },
        Err(e) => return exit_with_error(&request_state, e, ErrorKind::RecvAccessPoints),
    };

    let mut aps : Vec<AccessPointSerializable> = Vec::new();

    

    for ap in access_points {
        aps.push( AccessPointSerializable {
            ssid: ap.ssid().as_str().unwrap().to_string(),
            signal: ap.strength()
        } );
    }

    let output = serde_json::to_string( &aps );

    // Respond with list of SSIDs in JSON format
    Ok(Response::with((status::Ok, output.unwrap())))
}

fn connect(req: &mut Request) -> IronResult<Response> {
    let (ssid, passphrase) = {
        let params = get_request_ref!(req, Params, "Getting request params failed");
        let ssid = get_param!(params, "ssid", String);
        let passphrase = get_param!(params, "passphrase", String);
        (ssid, passphrase)
    };

    debug!("Incoming `connect` to access point `{}` request", ssid);

    let request_state = get_request_state!(req);

    let command = NetworkCommand::Connect {
        ssid: ssid,
        passphrase: passphrase,
    };

    if let Err(e) = request_state.network_tx.send(command) {
        exit_with_error(&request_state, e, ErrorKind::SendNetworkCommandConnect)
    } else {
        Ok(Response::with(status::Ok))
    }
}

fn disconnect(req: &mut Request) -> IronResult<Response> {

    let request_state = get_request_state!(req);

    let command = NetworkCommand::Disconnect;

    if let Err(e) = request_state.network_tx.send(command) {
        exit_with_error(&request_state, e, ErrorKind::SendNetworkCommandConnect)
    } else {
        Ok(Response::with(status::Ok))
    }
}

fn check_internet_connection(req: &mut Request) -> IronResult<Response> {

    let request_state = get_request_state!(req);
    let command = NetworkCommand::CheckInternet;

    // Send command to network thread to check internet connection
    if let Err(e) = request_state.network_tx.send(command) {
        return exit_with_error(&request_state, e, ErrorKind::PingUnsuccessful);
    }

    // Wait for network thread to respond
    let ping_result = match request_state.server_rx.recv() {
        Ok(result) => match result {
            NetworkCommandResponse::InternetCheckResponse(resp) => resp,
            _ => false
        },
        Err(e) => return exit_with_error(&request_state, e, ErrorKind::RecvAccessPointSSIDs),
    };

    // Send response
    match ping_result {
        true => Ok( Response::with(status::Ok) ),
        false => Ok( Response::with(status::ServiceUnavailable) )
    }
}

fn clear_connections(req: &mut Request) -> IronResult<Response> {

    let request_state = get_request_state!(req);
    let command = NetworkCommand::Clear;

    if let Err(e) = request_state.network_tx.send(command) {
        exit_with_error(&request_state, e, ErrorKind::SendNetworkCommandClear)
    } else {
        Ok(Response::with(status::Ok))
    }
}
