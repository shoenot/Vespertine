use alloc::{string::String, vec::Vec};
use vespertine_abi::{AccessRights, app::termios::Termios, tag::TAG_APP_TERM};
use vespertine_rt::println;
use vespertine_std::{
    Exec, env,
    term::{set_terminfo, unset_raw_mode},
};

pub fn launch<'a>(name: String, args: Vec<String>, rights: AccessRights) {
    let res = Exec::new(name.clone())
        .source(env::source())
        .sink(env::sink())
        .args(&args)
        .root_rights(rights)
        .inherit_capabilities()
        .spawn();

    match res {
        Ok(p) => {
            if let Err(e) = p.wait() {
                println!("[ERROR] {} error: {:?}", name, e);
            }
        }
        Err(e) => println!("{:?}", e),
    }

    let _ = unset_raw_mode();
}
