use log::{debug, error, info, warn};
use logger::init_logger;
use prost::Message;
use protos::{Opcode, Request};
use solana_sdk::{
    pubkey::Pubkey,
    sanitize::SanitizeError,
    transaction::{SanitizedVersionedTransaction, VersionedTransaction},
};
use std::{env, net::IpAddr, thread};
use std::{str::FromStr, thread::sleep, time::Duration};

use crate::protos::{response::Response, ResponseWrapper};

pub mod logger;

mod protos {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

const XAND_SHILED_KEY: &str = "xSHLJPXU8QW3A9kGiRoL94bksJ7ZZPY4dUwJPAT8CVK";

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut version_name: Option<&str> = None;
    let mut tcp_push_addr: Option<String> = None;
    let mut tcp_pull_addr: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" if i + 1 < args.len() => {
                version_name = match args[i + 1].to_lowercase().as_str() {
                    "vega" => Some("Vega"),
                    "altair" => Some("Altair"),
                    _ => {
                        eprintln!("Invalid version: {}", args[i + 1]);
                        print_usage_and_exit();
                    }
                };
                i += 2;
            }
            "--tcp-push" if i + 1 < args.len() => {
                tcp_push_addr = Some(format!("tcp://{}", args[i + 1]));
                i += 2;
            }
            "--tcp-pull" if i + 1 < args.len() => {
                tcp_pull_addr = Some(format!("tcp://{}", args[i + 1]));
                i += 2;
            }
            _ => {
                eprintln!("Unknown or incomplete argument: {}", args[i]);
                print_usage_and_exit();
            }
        }
    }

    let version_name = version_name.unwrap_or_else(|| {
        eprintln!("Missing --version argument.");
        print_usage_and_exit();
    });

    let tcp_push_addr = tcp_push_addr.unwrap_or_else(|| {
        eprintln!("Missing --tcp-push argument.");
        print_usage_and_exit();
    });

    let tcp_pull_addr = tcp_pull_addr.unwrap_or_else(|| {
        eprintln!("Missing --tcp-pull argument.");
        print_usage_and_exit();
    });

    init_logger(version_name).expect("Failed to initialize logger");

    let push_uds_path = "/var/run/xandeum/fromdock.sock";

    let pull_uds_path = "/var/run/xandeum/todock.sock";

    info!("UDS Pull Socket path : {}", pull_uds_path);
    info!("UDS Push Socket path : {}", push_uds_path);
    info!("Tcp Pull ip  : {}", tcp_pull_addr);
    info!("Tcp Push ip : {}", tcp_push_addr);

    let ip = local_ip_address::local_ip().unwrap();

    info!("ip address : {:?}", ip);

    let mut ip_bytes = [0u8; 16];

    match ip {
        IpAddr::V4(v4) => {
            ip_bytes[..4].copy_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            ip_bytes.copy_from_slice(&v6.octets());
        }
    }

    let context = zmq::Context::new();

    // Creating zmq sockets for the Back channel
    let uds_push_socket = context.socket(zmq::PUSH).unwrap();
    uds_push_socket
        .connect(&format!("ipc://{}", push_uds_path))
        .unwrap();

    let tcp_pull_socket = context.socket(zmq::XSUB).unwrap();
    tcp_pull_socket
        .connect(&tcp_pull_addr)
        .expect("Failed to bind PUB socket");

    tcp_pull_socket.send(b"\x01" as &[u8], 0).unwrap();

    // Starting atlas listener thread and Xandeum Agave Forwarder Thread
    thread::spawn(move || {
        info!("Starting Atlas Listener");
        loop {
            match tcp_pull_socket.recv_bytes(0) {
                Ok(msg) => {
                    debug!("Received Data from Atlas : {:?}", msg);

                    // Extracting IP address from the Response buffer
                    let (ip_prefix, req_bytes) = msg.split_at(16);
                    let ip_address = if ip_prefix[4..] == [0; 12] {
                        // It's IPv4
                        IpAddr::V4(std::net::Ipv4Addr::new(
                            ip_prefix[0],
                            ip_prefix[1],
                            ip_prefix[2],
                            ip_prefix[3],
                        ))
                    } else {
                        // It's IPv6
                        let mut octets = [0u8; 16];
                        octets.copy_from_slice(ip_prefix);
                        IpAddr::V6(std::net::Ipv6Addr::from(octets))
                    };

                    // Checking if the The Response is for a RPC request or a transaction
                    match ResponseWrapper::decode(req_bytes) {
                        Ok(wrapper) => {
                            let response = match wrapper.response {
                                Some(ref resp) => resp.clone(),
                                None => {
                                    error!(
                                        "Received empty Response in ResponseWrapper: id={}",
                                        wrapper.id
                                    );
                                    continue;
                                }
                            };

                            match response.response {
                                // If the response is for a RPC request and it is not originated from these docks/rpc
                                // the it should be discarded.Since Each RPC will have their own Request counter
                                // Request originated from other RPC will have different request number which could be
                                // higher from this rpc. If the response for that request is stored in this rpc then
                                // it will act as a poison response and it will provide false results in the future
                                // for that request number
                                Some(Response::Exists(_))
                                | Some(Response::ListDir(_))
                                | Some(Response::Metadata(_)) => {
                                    if ip != ip_address {
                                        info!("Received a RPC request originated from different RPC, discarding");
                                        continue;
                                    }
                                }
                                Some(Response::Tx(_)) => {}
                                None => {}
                            }
                        }
                        Err(e) => {
                            error!("Failed to decode ResponseWrapper: {:?}", e);
                            debug!("Raw message: {:?}", msg);
                        }
                    }

                    match uds_push_socket.send(req_bytes, 0) {
                        Ok(()) => debug!("Forwarded data from Atlas to UDS PUSH socket"),
                        Err(e) => error!("Failed to forward to UDS PUSH socket: {:?}", e),
                    }
                }
                Err(e) => {
                    error!("Error receiving from Atlas: {:?}", e);
                }
            }
        }
    });

    // Creating sockets To receive from Xandeum Agave And to send to Atlas
    let uds_pull_socket = context.socket(zmq::PULL).unwrap();
    uds_pull_socket
        .connect(&format!("ipc://{}", pull_uds_path))
        .unwrap();

    let tcp_push_socket = context.socket(zmq::PUSH).unwrap();
    tcp_push_socket
        .connect(&tcp_push_addr)
        .expect("Failed to bind PUB socket");

    // Listening to Xandeum agave for incoming Xtransactions and forwarding
    // them to Atlas
    loop {
        match uds_pull_socket.recv_bytes(0) {
            Ok(msg) => {
                let reqs = deserialize_requests(msg);

                info!("Request Received : {:?} ", reqs);

                if reqs.is_empty() {
                    warn!("No request Found, Skipping");
                }

                for req in reqs {
                    let mut buf = Vec::new();
                    req.encode(&mut buf).unwrap();

                    // Encoding first 16 bytes id buffer with Ipaddress so When it receives
                    // Response from Atlas We Receive The IP.
                    // This is needed For identifying RPC requests and their responses
                    // If the RPC request is originated from These Docks/RPC Then and only then
                    // It's response should be sent to the RPC else discarded

                    let mut final_buf = Vec::with_capacity(16 + buf.len());
                    final_buf.extend_from_slice(&ip_bytes);
                    final_buf.extend_from_slice(&buf);

                    let res = tcp_push_socket.send(&final_buf, 0);

                    match res {
                        Ok(()) => {
                            debug!("Sent a request : {:?}", req);
                        }
                        Err(e) => {
                            error!("Error Sending Req , Error : {}", e);
                        }
                    }
                }
            }
            Err(zmq::Error::EAGAIN) => {
                info!("No Message Received");
                sleep(Duration::from_millis(100));
            }
            Err(e) => {
                error!(
                    "Error occurred While Receiving Packets through UDS in zmq, Error : {:?}",
                    e
                );
            }
        }
    }
}

/// To Validate and process the XTransaction to a Request format
fn process_tx_to_proto_structure(tx: VersionedTransaction) -> Result<Vec<Request>, SanitizeError> {
    let mut reqs: Vec<Request> = Vec::new();
    let sanitized_tx = SanitizedVersionedTransaction::try_new(tx.clone())?;

    debug!("Sanitized Transaction : {:?}", sanitized_tx);
    let msg = sanitized_tx.get_message();

    let tx_hash = tx
        .signatures
        .get(0)
        .map(|sig| sig.to_string())
        .unwrap_or_else(|| {
            warn!("Transaction has no signatures, using default");
            "no-signature".to_string()
        });

    let xand_shield_pubkey =
        Pubkey::from_str(XAND_SHILED_KEY).expect("Invalid XAND_SHIELD_PROGRAM_ID");

    for (i, ix) in msg.instructions().iter().enumerate() {
        let pk = &msg.message.static_account_keys()[ix.program_id_index as usize];

        if *pk == xand_shield_pubkey {
            let signers: Vec<&Pubkey> = ix
                .accounts
                .iter()
                .filter_map(|&index| {
                    if msg.message.is_signer(index as usize) {
                        Some(&msg.message.static_account_keys()[index as usize])
                    } else {
                        None
                    }
                })
                .collect();

            if signers.is_empty() {
                warn!(
                    "Instruction {} has no signers, skipping request generation",
                    i
                );
                continue;
            }
            match ix.data.split_first() {
                Some((op, data)) => {
                    let opcode = match op {
                        0 => Opcode::Bigbang,
                        1 => Opcode::Armageddon,
                        2 => Opcode::Openrw,
                        3 => Opcode::Peek,
                        4 => Opcode::Poke,
                        5 => Opcode::Rm,
                        6 => Opcode::Mkdir,
                        7 => Opcode::Rmdir,
                        8 => Opcode::Rename,
                        9 => Opcode::Copy,
                        13 => Opcode::Move,
                        _ => {
                            warn!("Other Instructions are not supported yet, Skipping");
                            continue;
                        }
                    };
                    let req = Request {
                        op: opcode as i32,
                        pubkey: signers[0].to_bytes().to_vec(),
                        data: data.to_vec(),
                        signature: tx_hash.clone(),
                    };
                    reqs.push(req);
                }
                None => {
                    warn!(
                        "Instruction {} has empty data, skipping request generation",
                        i
                    );
                    continue;
                }
            }
        }
    }
    Ok(reqs)
}

fn print_usage_and_exit() -> ! {
    eprintln!("Usage: <binary> --version <vega|altair> --tcp-push <ip:port> --tcp-pull <ip:port>");
    std::process::exit(1);
}

fn deserialize_requests(msg: Vec<u8>) -> Vec<Request> {
    if let Ok(tx) = bincode::deserialize(&msg) {
        if let Ok(reqs) = process_tx_to_proto_structure(tx) {
            return reqs;
        }
    }

    match bincode::deserialize::<Request>(&msg) {
        Ok(r) => {
            vec![r]
        }
        Err(e) => {
            error!("Deserialization failed as both Tx and Request: {}", e);
            vec![]
        }
    }
}
