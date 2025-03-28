use log::info;
use logger::init_logger;
use prost::Message;
use solana_sdk::{
    pubkey::Pubkey,
    transaction::{SanitizedVersionedTransaction, VersionedTransaction},
};
use tokio::{net::UdpSocket, runtime::Runtime};
use std::{env, net::Ipv4Addr, thread};
use std::{str::FromStr, thread::sleep, time::Duration};
use types::{Opcode, Request};
pub mod logger;

mod types {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}
const PULL_UDS_PATH: &str = "/var/run/dock.sock";
const PUSH_UDS_PATH: &str = "/var/run/push.sock";
const TCP_PUSH_ADDR: &str = "tcp://167.86.82.28:8080";

const LOCAL_INTERFACE: &str = "0.0.0.0";
const UDP_PORT: u16 = 5500;
const MULTICAST_ADDR: &str = "tcp://167.86.82.28";

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

    let context = zmq::Context::new();

    let uds_pull_socket = context.socket(zmq::PULL).unwrap();
    uds_pull_socket
        .bind(&format!("ipc://{}", PULL_UDS_PATH))
        .unwrap();

    let tcp_push_socket = context.socket(zmq::PUSH).unwrap();
    tcp_push_socket
        .connect(TCP_PUSH_ADDR)
        .expect("Failed to bind PUB socket");

    let uds_push_socket = context.socket(zmq::PULL).unwrap();
    uds_pull_socket
        .bind(&format!("ipc://{}", PUSH_UDS_PATH))
        .unwrap();

    let tcp_pull_socket = match context.socket(zmq::PULL) {
        Ok(socket) => socket,
        Err(e) => {
            log::error!("Failed to create TCP PULL socket: {:?}", e);
            return;
        }
    };


    thread::spawn(move || {
        let rt = Runtime::new().expect("Failed to create Tokio runtime");

        rt.block_on(async move {
            info!("Starting the UDP Multicast Listener");

            // Create UDP socket
            let udp_socket = match UdpSocket::bind((LOCAL_INTERFACE, UDP_PORT)).await {
                Ok(sock) => {
                    info!("UDP socket bound to {}:{}", LOCAL_INTERFACE, UDP_PORT);
                    sock
                }
                Err(e) => {
                    log::error!("Failed to bind UDP socket: {:?}", e);
                    return;
                }
            };

            let multicast_addr: Ipv4Addr = MULTICAST_ADDR.parse().expect("Invalid multicast address");
            let local_ip: Ipv4Addr = LOCAL_INTERFACE.parse().expect("Invalid local interface IP");

            if let Err(e) = udp_socket.join_multicast_v4(multicast_addr, local_ip) {
                log::error!("Failed to join multicast group {}: {:?}", MULTICAST_ADDR, e);
                return;
            }

            info!("Joined multicast group: {}", MULTICAST_ADDR);

            let mut buf = [0u8; 1024];

            loop {
                match udp_socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let data = &buf[..len];
                        info!("Received {} bytes from {}: {:?}", len, addr, data);

                        match uds_push_socket.send(data, 0) {
                            Ok(()) => info!("Forwarded data from UDP to UDS PUSH socket"),
                            Err(e) => log::error!("Failed to forward to UDS PUSH socket: {:?}", e),
                        }
                    }
                    Err(e) => {
                        log::error!("Error receiving from UDP socket: {:?}", e);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });
    });

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
                sleep(Duration::from_millis(100));
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
    let sanitized_tx = SanitizedVersionedTransaction::try_new(tx).unwrap();

    log::info!("Sanitized Transaction : {:?}", sanitized_tx);
    let msg = sanitized_tx.get_message();

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
                Some((op, data)) => match op {
                    0 => {
                        let req = Request {
                            op: Opcode::Bigbang as i32,
                            pubkey: signers[0].to_bytes().to_vec(),
                            data: data.to_vec(),
                        };
                        reqs.push(req);
                    }
                    1 => {
                        let req = Request {
                            op: Opcode::Armageddon as i32,
                            pubkey: signers[0].to_bytes().to_vec(),
                            data: data.to_vec(),
                        };
                        reqs.push(req);
                    }
                    _ => {
                        log::warn!("Other Instruction  are not supported yet, Skipping");
                        continue;
                    }
                },
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
