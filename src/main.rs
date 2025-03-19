use logger::init_logger;
use prost::Message;
use solana_sdk::{
    pubkey::Pubkey,
    transaction::{SanitizedVersionedTransaction, VersionedTransaction},
};
use std::env;
use std::{str::FromStr, thread::sleep, time::Duration};
use types::{Opcode, Request};
pub mod logger;

mod types {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}
const UDS_PATH: &str = "/var/run/dock.sock";
const TCP_ADDR: &str = "tcp://167.86.82.28:8080";
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
    let socket = context.socket(zmq::PULL).unwrap();

    socket.bind(&format!("ipc://{}", UDS_PATH)).unwrap();
    //    socket.set_subscribe(b"").unwrap();

    let pub_socket = context.socket(zmq::PUB).unwrap();
    pub_socket
        .connect(TCP_ADDR)
        .expect("Failed to bind PUB socket");

    log::info!("Receiving data from UDS socket ");

    loop {
        match socket.recv_bytes(0) {
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

                    let res = pub_socket.send(&buf, 0);

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
