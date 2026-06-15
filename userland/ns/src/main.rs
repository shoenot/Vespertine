#![no_std]
#![no_main]
use alloc::format;
use alloc::string::String;
use vespertine_abi::AccessRights;
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::tag::*;
use vespertine_rt::print;
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_rt::syscall::sys_sleep;
use vespertine_rt::thread as rt_thread;
use vespertine_std::Error;
use vespertine_std::ErrorKind;
use vespertine_std::Read;
use vespertine_std::Write;
use vespertine_std::env;
use vespertine_std::fs::Dir;
use vespertine_std::fs::File;
use vespertine_std::fs::Path;
use vespertine_std::fs::resolve;
extern crate alloc;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] ns error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let args = env::args();

    if args.len() < 2 {
        return Err(Error {
            kind: ErrorKind::InvalidArgument,
            message: "ns needs an operation to perform".into(),
        });
    }

    let optional_args = if args.len() > 2 {
        Some(args[2].clone())
    } else {
        None
    };

    let opstr = args[1].as_str();

    match opstr {
        "list" | "ls" => {
            let dir = if let Some(path) = optional_args {
                Dir::open(&Path::new(path.as_str()))?
            } else {
                Dir::open(&Path::new("."))?
            };
            let mut dir_iter = dir.list()?;
            while let Some(entry) = dir_iter.next() {
                if entry != "lost+found" {
                    println!("{}", entry);
                }
            }
        }
        "read" | "cat" => {
            let errmsg = String::from(format!("{} needs a directory path to create", opstr));
            if let Some(filepath) = optional_args {
                let filestr = File::open(&Path::new(filepath.as_str()))?;
                print_stream(&filestr)?;
            } else {
                return Err(Error {
                    kind: ErrorKind::InvalidArgument,
                    message: errmsg,
                });
            }
        }
        "newdir" | "mkdir" => {
            let errmsg = String::from(format!("{} needs a directory path to create", opstr));
            let path = optional_args.ok_or(Error {
                kind: ErrorKind::InvalidArgument,
                message: errmsg,
            })?;
            Dir::create_dir(&Path::new(path.as_str()))?;
            println!("Directory created successfully");
        }
        "newfile" | "touch" => {
            let errmsg = String::from(format!("{} needs a path to create a file at", opstr));
            let path = optional_args.ok_or(Error {
                kind: ErrorKind::InvalidArgument,
                message: errmsg,
            })?;
            File::create(&Path::new(path.as_str()))?;
            println!("File created successfully");
        }
        "delete" | "rm" => {
            let errmsg = String::from(format!("{} needs a path to remove", opstr));
            let path = optional_args.ok_or(Error {
                kind: ErrorKind::InvalidArgument,
                message: errmsg,
            })?;
            Dir::remove(&Path::new(path.as_str()))?;
            println!("File created successfully");
        }
        "write" => {
            if args.len() < 4 {
                return Err(Error {
                    kind: ErrorKind::InvalidArgument,
                    message: "write needs a path and the text content to write".into(),
                });
            }
            let path = &args[2];
            let mut content = alloc::string::String::new();
            for (i, arg) in args.iter().enumerate().skip(3) {
                if i > 3 {
                    content.push(' ');
                }
                content.push_str(arg);
            }
            let trimmed_content =
                if content.starts_with('"') && content.ends_with('"') && content.len() >= 2 {
                    &content[1..content.len() - 1]
                } else {
                    &content
                };

            let file = File::open_with_rights(&Path::new(path.as_str()), AccessRights::WRITE)?;
            file.write_all(trimmed_content.as_bytes())?;
            println!("File written successfully");
        }
        _ => {
            return Err(Error {
                kind: ErrorKind::InvalidArgument,
                message: "Invalid Operation".into(),
            });
        }
    }
    Ok(())
}

pub fn print_stream<R: Read>(stream: &R) -> Result<(), Error> {
    let text = stream.read_to_string()?;
    print!("{}", text);
    Ok(())
}
