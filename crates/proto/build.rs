use std::path::Path;

fn main() {
    // Vendored protoc keeps the build hermetic: teammates do not need a system
    // protobuf install, and everyone generates with the same compiler.
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc binary");
    std::env::set_var("PROTOC", protoc);

    let proto_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../proto")
        .canonicalize()
        .expect("proto/ directory at the workspace root");
    let file = proto_root.join("something/v1/something.proto");

    println!("cargo:rerun-if-changed={}", file.display());

    prost_build::compile_protos(&[&file], &[&proto_root]).expect("compile something.proto");
}
