fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc vendored path");
    // SAFETY: build scripts run single-process; env var is required so prost/tonic
    // can locate the vendored protoc binary.
    unsafe { std::env::set_var("PROTOC", protoc) };

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/agent.proto", "proto/raft.proto"], &["proto"])
        .expect("compile protos");

    println!("cargo:rerun-if-changed=proto/agent.proto");
    println!("cargo:rerun-if-changed=proto/raft.proto");
}
