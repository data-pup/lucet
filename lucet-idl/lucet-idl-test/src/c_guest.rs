use crate::workspace::Workspace;
use failure::{format_err, Error};
use lucet_idl::{self, Backend, Config, Package};
use lucet_wasi;
use lucetc::{Lucetc, LucetcOpts};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

pub struct CGuestApp {
    work: Workspace,
}

impl CGuestApp {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            work: Workspace::new()?,
        })
    }

    fn generate_idl_h(&mut self, package: &Package) -> Result<(), Error> {
        lucet_idl::codegen(
            package,
            &Config {
                backend: Backend::CGuest,
            },
            Box::new(File::create(self.work.source_path("idl.h"))?),
        )?;
        Ok(())
    }

    fn generate_main_c(&mut self) -> Result<(), Error> {
        let mut main_file = File::create(self.work.source_path("main.c"))?;
        main_file.write_all(
            b"
#include <stdio.h>
#include \"idl.h\"

int main(int argc, char* argv[]) {
    printf(\"hello, world from c guest\");
}",
        )?;
        Ok(())
    }
    fn wasi_clang(&mut self) -> Result<(), Error> {
        let wasi_sdk =
            PathBuf::from(std::env::var("WASI_SDK").unwrap_or_else(|_| "/opt/wasi-sdk".to_owned()));
        let cmd_cc = Command::new(wasi_sdk.join("bin").join("clang"))
            .arg("--std=c99")
            .arg(self.work.source_path("main.c"))
            .arg("-I")
            .arg(self.work.source_path(""))
            .arg("-o")
            .arg(self.work.output_path("out.wasm"))
            .status()?;

        if !cmd_cc.success() {
            Err(format_err!("clang error building guest"))?
        }
        Ok(())
    }

    pub fn build(&mut self, package: &Package) -> Result<PathBuf, Error> {
        self.generate_idl_h(package)?;
        self.generate_main_c()?;
        self.wasi_clang()?;
        let lucetc =
            Lucetc::new(self.work.output_path("out.wasm")).with_bindings(lucet_wasi::bindings());
        let so_file = self.work.output_path("out.so");
        lucetc.shared_object_file(&so_file)?;
        Ok(so_file)
    }
}
