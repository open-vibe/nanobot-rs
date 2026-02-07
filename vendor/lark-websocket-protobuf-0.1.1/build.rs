use std::io::Result;

fn main() -> Result<()> {
    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("failed to locate vendored protoc binary");
    std::env::set_var("PROTOC", protoc);
    prost_build::compile_protos(&["protos/pbbp2.proto"], &["protos/"])?;

    Ok(())
}
