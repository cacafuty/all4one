fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc vendored path");
    // SAFETY: build scripts run in a single process context and setting an env var here
    // is required so prost/tonic can locate the vendored protoc binary.
    unsafe { std::env::set_var("PROTOC", protoc) };

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/agent.proto"], &["proto"])
        .expect("compile protos");

    println!("cargo:rerun-if-changed=proto/agent.proto");
}
