fn main() {
    prost_build::compile_protos(&["test-protos/types.proto"], &["test-protos"])
        .expect("Failed to compile Protobuf file");
}
