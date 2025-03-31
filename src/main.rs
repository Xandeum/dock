use log::info;
use logger::init_logger;
use prost::Message;
use solana_sdk::{
    pubkey::Pubkey,
    transaction::{SanitizedVersionedTransaction, VersionedTransaction},
};
use std::{env, net::Ipv4Addr, thread};
use std::{str::FromStr, thread::sleep, time::Duration};
use tokio::{net::UdpSocket, runtime::Runtime};
use types::{Opcode, Request};
use zmq::DONTWAIT;
pub mod logger;

mod types {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}
// const PULL_UDS_PATH: &str = "/tmp/dock.sock";
// const PUSH_UDS_PATH: &str = "/tmp/push.sock";
const TCP_PUSH_ADDR: &str = "tcp://65.108.233.175:8080";
const TCP_PULL_ADDR: &str = "tcp://65.108.233.175:8081";

// const LOCAL_INTERFACE: &str = "0.0.0.0";
// // const LOCAL_INTERFACE: &str = "65.108.233.175";
// const UDP_PORT: u16 = 8081;
// const MULTICAST_ADDR: &str = "239.1.1.1";

const XAND_SHILED_KEY: &str = "xSHLJPXU8QW3A9kGiRoL94bksJ7ZZPY4dUwJPAT8CVK";

fn main() {
    let args: Vec<String> = env::args().collect();
    let version_name = if args.len() > 2 && args[1] == "--version" {
        match args[2].as_str() {
            "vega" => "Vega",
            "altair" => "Altair",
            _ => {
                println!(
                    "Invalid version. Use --version vega or --version altair. Defaulting to Vega."
                );
                "Vega"
            }
        }
    } else {
        println!(
            "No version specified. Use --version vega or --version altair. Defaulting to Vega."
        );
        "Vega"
    };

    init_logger(version_name).expect("Failed to initialize logger");

    let push_uds_path = match version_name {
        "Vega" => "/var/run/xandeum/vega_pull.sock",
        "Altair" => "/var/run/xandeum/altair_pull.sock",
        _ => unreachable!(),
    };

    let pull_uds_path = match version_name {
        "Vega" => "/var/run/xandeum/vega.sock",
        "Altair" => "/var/run/xandeum/altair.sock",
        _ => unreachable!(),
    };

    log::info!("UDS Pull Socket path : {}", pull_uds_path);
    log::info!("UDS Push Socket path : {}", push_uds_path);
    log::info!("Tcp Pull ip  : {}", TCP_PULL_ADDR);
    log::info!("Tcp Push ip : {}", TCP_PUSH_ADDR);

    let context = zmq::Context::new();

    let uds_push_socket = context.socket(zmq::PUSH).unwrap();
    uds_push_socket
        .connect(&format!("ipc://{}", push_uds_path))
        .unwrap();

    let tcp_pull_socket = context.socket(zmq::XSUB).unwrap();
    tcp_pull_socket
        .connect(TCP_PULL_ADDR)
        .expect("Failed to bind PUB socket");

    tcp_pull_socket.send(b"\x01" as &[u8], 0).unwrap();

    thread::spawn(move || {
        info!("Starting the UDP Multicast Listener");

        loop {
            match tcp_pull_socket.recv_bytes(0) {
                Ok(msg) => {
                    info!("Received bytes from Atlas : {:?}", msg);

                    match uds_push_socket.send(msg, 0) {
                        Ok(()) => info!("Forwarded data from UDP to UDS PUSH socket"),
                        Err(e) => log::error!("Failed to forward to UDS PUSH socket: {:?}", e),
                    }
                }
                Err(e) => {
                    log::error!("Error receiving from UDP socket: {:?}", e);
                }
            }
        }
    });

    let uds_pull_socket = context.socket(zmq::PULL).unwrap();
    uds_pull_socket
        .connect(&format!("ipc://{}", pull_uds_path))
        .unwrap();

    let tcp_push_socket = context.socket(zmq::PUSH).unwrap();
    tcp_push_socket
        .connect(TCP_PUSH_ADDR)
        .expect("Failed to bind PUB socket");

    loop {
        match uds_pull_socket.recv_bytes(0) {
            Ok(msg) => {
                let tx: VersionedTransaction = bincode::deserialize(&msg).unwrap();
                log::info!("received tx from Rpc : {:?}", msg);
                let reqs = process_tx_to_proto_structure(tx);

                if reqs.is_empty() {
                    log::warn!("No request Found, Skipping");
                }

                for req in reqs {
                    let mut buf = Vec::new();
                    req.encode(&mut buf).unwrap();

                    let res = tcp_push_socket.send(&buf, 0);

                    match res {
                        Ok(()) => {
                            log::debug!("Sent a request : {:?}", req);
                        }
                        Err(e) => {
                            log::error!("Error Sending Req , Error : {}", e);
                        }
                    }
                }
            }
            Err(zmq::Error::EAGAIN) => {
                log::info!("No Message Received");
                sleep(Duration::from_millis(500));
            }
            Err(e) => {
                log::error!(
                    "Error occurred While Receiving Packets through UDS in zmq, Error : {:?}",
                    e
                );
            }
        }
    }
}

fn process_tx_to_proto_structure(tx: VersionedTransaction) -> Vec<Request> {
    let mut reqs: Vec<Request> = Vec::new();
    let sanitized_tx = SanitizedVersionedTransaction::try_new(tx.clone()).unwrap();

    log::info!("Sanitized Transaction : {:?}", sanitized_tx);
    let msg = sanitized_tx.get_message();

    let tx_hash = tx
        .signatures
        .get(0)
        .map(|sig| sig.to_string()) // Convert to base58 string
        .unwrap_or_else(|| {
            log::warn!("Transaction has no signatures, using default");
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
                log::warn!(
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
                        _ => {
                            log::warn!("Other Instructions are not supported yet, Skipping");
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
                    log::warn!(
                        "Instruction {} has empty data, skipping request generation",
                        i
                    );
                    continue;
                }
            }
        }
    }
    reqs
}
