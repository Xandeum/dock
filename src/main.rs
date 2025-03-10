use std::{str::FromStr, thread::sleep, time::Duration};

use logger::init_logger;
use prost::Message;
use solana_sdk::{
    message::{SanitizedMessage, SanitizedVersionedMessage},
    pubkey::Pubkey,
    transaction::{SanitizedTransaction, SanitizedVersionedTransaction, VersionedTransaction},
};
use types::{Opcode, Request};
pub mod logger;

mod types {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

const UDS_PATH: &str = "/var/run/xandeum/dock.sock";
const TCP_ADDR: &str = "tcp://0.0.0.0:8080";

fn main() {
    init_logger().expect("Failed to initialize logger");

    let context = zmq::Context::new();
    let socket = context.socket(zmq::SUB).unwrap();

    socket.connect(&format!("ipc://{}", UDS_PATH)).unwrap();
    socket.set_subscribe(b"").unwrap();

    let pub_socket = context.socket(zmq::PUB).unwrap();
    pub_socket
        .bind(TCP_ADDR)
        .expect("Failed to bind PUB socket");

    log::info!("Receiving data from UDS socket ");

    loop {
        match socket.recv_bytes(zmq::DONTWAIT) {
            Ok(msg) => {
                let tx: VersionedTransaction = bincode::deserialize(&msg).unwrap();

                let reqs = process_tx_to_proto_structure(tx);

                for req in reqs {
                    let mut buf = Vec::new();
                    req.encode(&mut buf).unwrap();

                    pub_socket.send(&buf, 0).expect("Failed to send via tcp")
                }
            }
            Err(zmq::Error::EAGAIN) => {
                log::info!("No Message Received");
                sleep(Duration::from_secs(5));
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

    let msg = sanitized_tx.get_message();

    let xand_shield_pubkey = Pubkey::from_str("5454").expect("Invalid XAND_SHIELD_PROGRAM_ID");

    // let xand_shield_index = msg
    //     .message
    //     .static_account_keys()
    //     .iter()
    //     .position(|key| key == &xand_shield_pubkey);

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

            let data = ix.data.split_at(1);

            let req = Request {
                op: Opcode::Armageddon as i32,
                pubkey: signers[0].to_bytes().to_vec(),
                data: data.1.to_vec(),
            };
            reqs.push(req);
        }
    }
    reqs
}
