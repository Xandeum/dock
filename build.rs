fn main() {
    prost_build::compile_protos(&["xandeum-protos/types.proto","xandeum-protos/response.proto"], &["xandeum-protos"])
        .expect("Failed to compile Protobuf file");
}
