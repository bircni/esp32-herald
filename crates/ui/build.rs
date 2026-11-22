//! Gzips the web app at build time, so the device serves ~12 KB instead of
//! ~45 KB, and derives an `ETag` from its content so reloads cost nothing.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let src = Path::new("web/index.html");
    println!("cargo::rerun-if-changed={}", src.display());
    let html = fs::read(src)?;
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(&html)?;
    let gz = gz.finish()?;

    let out = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out.join("index.html.gz"), &gz)?;
    fs::write(
        out.join("web.rs"),
        format!(
            "/// The web app, gzipped.\n\
             pub const INDEX_GZ: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/index.html.gz\"));\n\
             /// Changes whenever the page does.\n\
             pub const INDEX_ETAG: &str = \"\\\"{:016x}\\\"\";\n",
            fnv(&html)
        ),
    )?;
    Ok(())
}
