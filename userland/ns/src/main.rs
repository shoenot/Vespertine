#![no_std]
#![no_main]
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::tag::*;
use vespertine_rt::print;
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_std::Error;
use vespertine_std::ErrorKind;
use vespertine_std::Read;
use vespertine_std::env;
use vespertine_std::fs::Dir;
use vespertine_std::fs::DirEntry;
use vespertine_std::fs::File;
extern crate alloc;
use alloc::vec::Vec;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] ns error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let sf = env::find_tag(TAG_SYS_SOCKFAC)
        .ok_or(Error { kind: ErrorKind::AccessDenied, message: "Socket Factory capability not found" })?;

    let args = env::args();

    if args.len() < 2 { 
        return Err(Error { kind: ErrorKind::InvalidArgument, message: "ns needs an operation to perform" }) 
    }
    
    let optional_args = if args.len() > 2 { Some(args[2].clone()) } else { None };
    
    match args[1].as_str() {
         "list" => {
            let dir = if optional_args.is_none() { 
                Dir::from(env::root()) 
            } else { 
                Dir::open(optional_args.unwrap().as_str())?
            };
            let mut dir_iter = dir.list()?;
            while let Some(entry) = dir_iter.next() {
                println!("{}", entry);
            }
        },
        "read" => {
            let file = File::open(optional_args.unwrap().as_str())?;
            print_stream(&file)?;
        }
        _ => return Err(Error { kind: ErrorKind::InvalidArgument, message: "Invalid Operation" }),
    }
    Ok(())
}

pub fn print_stream<R: Read>(stream: &R) -> Result<(), Error> {
    let text = stream.read_to_string()?;
    print!("{}", text);
    Ok(())
}
