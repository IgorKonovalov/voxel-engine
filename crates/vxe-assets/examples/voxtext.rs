//! Voxels drawn as text to a `.vox` and back.
//!
//! ```text
//! cargo run -p vxe-assets --example voxtext -- to-vox in.voxtext out.vox
//! cargo run -p vxe-assets --example voxtext -- from-vox in.vox out.voxtext
//! ```

use std::process::ExitCode;

use vxe_assets::{VoxFile, VoxText, encode};

fn run(args: &[String]) -> Result<(), String> {
    let [cmd, input, output] = args else {
        return Err(
            "usage: voxtext to-vox <in.voxtext> <out.vox> | from-vox <in.vox> <out.voxtext>".into(),
        );
    };
    match cmd.as_str() {
        "to-vox" => {
            let text = std::fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
            let vt = VoxText::parse(&text).map_err(|e| format!("{input}: {e}"))?;
            std::fs::write(output, encode(&vt.to_vox_file())).map_err(|e| format!("{output}: {e}"))
        }
        "from-vox" => {
            let bytes = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
            let vox = VoxFile::parse(&bytes).map_err(|e| format!("{input}: {e}"))?;
            let vt = VoxText::from_vox_file(&vox).map_err(|e| format!("{input}: {e}"))?;
            std::fs::write(output, vt.write()).map_err(|e| format!("{output}: {e}"))
        }
        _ => Err(format!("unknown command `{cmd}`: to-vox or from-vox")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("voxtext: {e}");
            ExitCode::FAILURE
        }
    }
}
