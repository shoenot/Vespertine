use alloc::{collections::btree_map::BTreeMap, string::String, vec::Vec};
use vespertine_abi::{ProcInfo, UserID, typed::{FileSizeValue, UserValue, ValueType}};
use vespertine_cli::args::Command;
use vespertine_rt::println;
use vespertine_std::{Error, auth::AuthClient, list_processes, typed::{RecordStream, TypedValue, user_value}};

pub const SYS_PROCS_LIST_SCHEMA: u64 = 1;

pub fn procs(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("list")
        .parse(args).
        map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: sys procs");
        return Ok(());
    }

    if matches.positional_count() > 0 {
        return Err(Error::invalid_argument("usage: sys procs".into()));
    }

    let entries = list_processes()?.collect::<Vec<_>>();
    let users = resolve_users(&entries)?;

    let mut out = RecordStream::typed_default_out(
        SYS_PROCS_LIST_SCHEMA,
        &[
            ("pid", ValueType::Integer),
            ("name", ValueType::String),
            ("user", ValueType::User),
            ("state", ValueType::String),
            ("threads", ValueType::Integer),
            ("memory", ValueType::FileSize),
            ("reason", ValueType::Integer),
            ("code", ValueType::Integer),
            ("detail", ValueType::Integer),
        ],
        &["pid", "name", "state"],
    )?;
    out.list_intent()?;
    out.table(&["pid", "name", "user", "state", "threads", "memory", "reason", "code", "detail"])?;

    for entry in entries {
        let user = users.get(&entry.user.0).copied().unwrap_or_else(|| user_value(entry.user.0));
        out.row_values(&[
            TypedValue::Integer(entry.pid as i128),
            TypedValue::String(entry.name().into()),
            TypedValue::User(user),
            TypedValue::String(entry.short_status().into()),
            TypedValue::Integer(entry.active_threads as i128),
            TypedValue::FileSize(FileSizeValue {
                bytes: entry.memory_usage as i128,
                block_size: 0, blocks: 0, flags: 0, reserved: 0,
            }),
            TypedValue::Integer(entry.term_reason as i128),
            TypedValue::Integer(entry.term_code as i128),
            TypedValue::Integer(entry.term_detail as i128),
        ])?;
    }

    out.finish()?;

    Ok(())
}

fn resolve_users(entries: &[ProcInfo]) -> Result<BTreeMap<u32, UserValue>, Error> {
    let mut users = BTreeMap::new();
    for entry in entries {
        if users.contains_key(&entry.user.0) {
            continue;
        }
        users.insert(entry.user.0, user_value(entry.user.0));
    }

    let mut auth = AuthClient::connect()?;
    for user_id in users.keys().copied().collect::<Vec<_>>() {
        let account = auth.lookup_id(UserID(user_id))?;
        users.insert(user_id, account.user);
    }
    Ok(users)
}
