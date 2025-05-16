// fn main() {
//     prost_build::compile_protos(&["xandeum-protos/types.proto","xandeum-protos/response.proto"], &["xandeum-protos"])
//         .expect("Failed to compile Protobuf file");
// }


use std::env;
use prost_build::Config;

fn main() {
    let mut config = Config::new();

    config.type_attribute(
        ".",
        "#[derive(serde::Serialize, serde::Deserialize)]",
    );

    config.extern_path(".serde", "::serde");

    // Print current directory to debug issues
    println!("cargo:warning=Current directory: {:?}", env::current_dir().unwrap());

    config
        .compile_protos(
            &["xandeum-protos/response.proto", "xandeum-protos/types.proto"], // Check if these paths exist
            &["xandeum-protos"],
        )
        .expect("Failed to compile Protobuf files");
}
