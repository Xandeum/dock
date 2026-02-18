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
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

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
    let mut num_workers: usize = 4; // Default to 4 workers

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
            "--workers" if i + 1 < args.len() => {
                num_workers = match args[i + 1].parse::<usize>() {
                    Ok(n) if n > 0 && n <= 32 => n,
                    _ => {
                        eprintln!("Invalid number of workers: {} (must be 1-32)", args[i + 1]);
                        print_usage_and_exit();
                    }
                };
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
    info!("Worker threads: {}", num_workers);

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

    let context = Arc::new(zmq::Context::new());
    
    // Create channels for Atlas → Agave direction
    let (atlas_sender, atlas_receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(1000);
    
    // Create channels for Agave → Atlas direction
    let (agave_sender, agave_receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(1000);

    // Atlas → Agave: Receiver thread
    let tcp_pull_socket = context.socket(zmq::XSUB).unwrap();
    tcp_pull_socket
        .connect(&tcp_pull_addr)
        .expect("Failed to connect XSUB socket");
    tcp_pull_socket.send(b"\x01" as &[u8], 0).unwrap();
    
    let atlas_sender_clone = atlas_sender.clone();
    thread::spawn(move || {
        info!("Starting Atlas Receiver");
        loop {
            match tcp_pull_socket.recv_bytes(0) {
                Ok(msg) => {
                    if let Err(e) = atlas_sender_clone.send(msg) {
                        error!("Failed to send to Atlas channel: {:?}", e);
                    }
                }
                Err(e) => {
                    error!("Error receiving from Atlas: {:?}", e);
                }
            }
        }
    });

    // Atlas → Agave: Worker threads
    for worker_id in 0..num_workers {
        let atlas_receiver_clone = atlas_receiver.clone();
        let context_clone = Arc::clone(&context);
        let ip_clone = ip.clone();
        let push_uds_path_clone = push_uds_path.to_string();
        
        thread::spawn(move || {
            info!("Starting Atlas → Agave worker {}", worker_id);
            
            // Each worker has its own UDS push socket
            let uds_push_socket = context_clone.socket(zmq::PUSH).unwrap();
            uds_push_socket
                .connect(&format!("ipc://{}", push_uds_path_clone))
                .unwrap();
            
            loop {
                match atlas_receiver_clone.recv() {
                    Ok(msg) => {
                        debug!("Worker {} processing Atlas message", worker_id);
                        
                        // Extracting IP address from the Response buffer
                        let (ip_prefix, req_bytes) = msg.split_at(16);
                        let ip_address = if ip_prefix[4..] == [0; 12] {
                            IpAddr::V4(std::net::Ipv4Addr::new(
                                ip_prefix[0],
                                ip_prefix[1],
                                ip_prefix[2],
                                ip_prefix[3],
                            ))
                        } else {
                            let mut octets = [0u8; 16];
                            octets.copy_from_slice(ip_prefix);
                            IpAddr::V6(std::net::Ipv6Addr::from(octets))
                        };

                        // Checking if the Response is for a RPC request or a transaction
                        match ResponseWrapper::decode(req_bytes) {
                            Ok(wrapper) => {
                                let response = match wrapper.response {
                                    Some(ref resp) => resp.clone(),
                                    None => {
                                        error!(
                                            "Worker {}: Received empty Response in ResponseWrapper: id={}",
                                            worker_id, wrapper.id
                                        );
                                        continue;
                                    }
                                };

                                match response.response {
                                    Some(Response::Exists(_))
                                    | Some(Response::ListDir(_))
                                    | Some(Response::Metadata(_)) => {
                                        if ip_clone != ip_address {
                                            info!("Worker {}: Received RPC request from different RPC, discarding", worker_id);
                                            continue;
                                        }
                                    }
                                    Some(Response::Tx(_)) => {}
                                    None => {}
                                }
                            }
                            Err(e) => {
                                error!("Worker {}: Failed to decode ResponseWrapper: {:?}", worker_id, e);
                                debug!("Worker {}: Raw message: {:?}", worker_id, msg);
                            }
                        }

                        match uds_push_socket.send(req_bytes, 0) {
                            Ok(()) => debug!("Worker {}: Forwarded data to UDS PUSH socket", worker_id),
                            Err(e) => error!("Worker {}: Failed to forward to UDS PUSH socket: {:?}", worker_id, e),
                        }
                    }
                    Err(e) => {
                        error!("Worker {}: Channel receive error: {:?}", worker_id, e);
                        break;
                    }
                }
            }
        });
    }

    // Agave → Atlas: Receiver thread
    let uds_pull_socket = context.socket(zmq::PULL).unwrap();
    uds_pull_socket
        .connect(&format!("ipc://{}", pull_uds_path))
        .unwrap();
    
    let agave_sender_clone = agave_sender.clone();
    thread::spawn(move || {
        info!("Starting Agave Receiver");
        loop {
            match uds_pull_socket.recv_bytes(0) {
                Ok(msg) => {
                    if let Err(e) = agave_sender_clone.send(msg) {
                        error!("Failed to send to Agave channel: {:?}", e);
                    }
                }
                Err(zmq::Error::EAGAIN) => {
                    // Non-blocking receive, continue
                    thread::yield_now();
                }
                Err(e) => {
                    error!("Error receiving from Agave: {:?}", e);
                }
            }
        }
    });

    // Agave → Atlas: Worker threads
    for worker_id in 0..num_workers {
        let agave_receiver_clone = agave_receiver.clone();
        let context_clone = Arc::clone(&context);
        let ip_bytes_clone = ip_bytes.clone();
        let tcp_push_addr_clone = tcp_push_addr.clone();
        
        thread::spawn(move || {
            info!("Starting Agave → Atlas worker {}", worker_id);
            
            // Each worker has its own TCP push socket
            let tcp_push_socket = context_clone.socket(zmq::PUSH).unwrap();
            tcp_push_socket
                .connect(&tcp_push_addr_clone)
                .expect("Failed to connect PUSH socket");
            
            loop {
                match agave_receiver_clone.recv() {
                    Ok(msg) => {
                        info!("Worker {} processing Agave message", worker_id);
                        
                        let reqs = deserialize_requests(msg);
                        
                        if reqs.is_empty() {
                            warn!("Worker {}: No request found, skipping", worker_id);
                            continue;
                        }
                        
                        // Process multiple requests in batch
                        for req in reqs {
                            let mut buf = Vec::new();
                            req.encode(&mut buf).unwrap();
                            
                            let mut final_buf = Vec::with_capacity(16 + buf.len());
                            final_buf.extend_from_slice(&ip_bytes_clone);
                            final_buf.extend_from_slice(&buf);
                            
                            match tcp_push_socket.send(&final_buf, 0) {
                                Ok(()) => {
                                    info!("Worker {}: Sent request: {:?}", worker_id, req);
                                }
                                Err(e) => {
                                    error!("Worker {}: Error sending request: {}", worker_id, e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Worker {}: Channel receive error: {:?}", worker_id, e);
                        break;
                    }
                }
            }
        });
    }
    
    // Set up signal handling for graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    
    ctrlc::set_handler(move || {
        info!("Received shutdown signal, stopping...");
        r.store(false, Ordering::SeqCst);
    }).expect("Error setting Ctrl-C handler");
    
    // Keep main thread alive and monitor shutdown signal
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_secs(1));
    }
    
    info!("Shutting down gracefully...");
    // Give workers time to finish current tasks
    thread::sleep(Duration::from_secs(2));
    info!("Shutdown complete");
}

/// To Validate and process the XTransaction to a Request format
fn process_tx_to_proto_structure(tx: VersionedTransaction) -> Result<Vec<Request>, SanitizeError> {
    let mut reqs: Vec<Request> = Vec::new();
    let sanitized_tx = SanitizedVersionedTransaction::try_new(tx.clone())?;

    info!("Sanitized Transaction : {:?}", sanitized_tx);
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
            match ix.data.get(1) {
                Some(op) => {
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
                        14 => Opcode::AssignCoowner,
                        16 => Opcode::Find,
                        _ => {
                            warn!("Other Instructions are not supported yet, Skipping");
                            continue;
                        }
                    };
                    let data = &ix.data[2..];
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
    eprintln!("Usage: <binary> --version <vega|altair> --tcp-push <ip:port> --tcp-pull <ip:port> [--workers <1-32>]");
    eprintln!("\nOptions:");
    eprintln!("  --version     Version name (vega or altair)");
    eprintln!("  --tcp-push    TCP address for pushing to Atlas");
    eprintln!("  --tcp-pull    TCP address for pulling from Atlas");
    eprintln!("  --workers     Number of worker threads (default: 4, max: 32)");
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
