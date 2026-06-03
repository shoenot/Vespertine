use core::cmp;
use core::ptr::copy_nonoverlapping;
use alloc::boxed::Box;

use alloc::{collections::btree_map::BTreeMap, string::{String, ToString}, sync::Arc, vec::Vec};
use async_trait::async_trait;
use vespertine_abi::{AccessRights, DirectoryOp, FileOp, Invocation, protocol::{AbiDirEntry, DirEntryType, PacketFlags, PacketHeader, VESPER_MAGIC}};

use crate::core::{object::{invoke::InvocationError, models::directory::Filename, obj::KernelObject}, sync::RwLock, thread::get_current_process};

#[derive(Debug)]
pub struct MountDirectory {
    pub underlying: RwLock<Arc<dyn KernelObject>>,
    pub overlays: RwLock<BTreeMap<Filename, Arc<dyn KernelObject>>>,
}

impl MountDirectory {
    pub fn new(underlying: Arc<dyn KernelObject>) -> Self {
        Self { 
            underlying: RwLock::new(underlying), 
            overlays: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn set_underlying(&self, new_underlying: Arc<dyn KernelObject>) {
        *self.underlying.write() = new_underlying;
    }
}

#[async_trait]
impl KernelObject for MountDirectory {
    fn type_name(&self) -> &'static str {
        "Directory"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Directory(DirectoryOp::Lookup { name, name_len }) => {
                let filename = Filename::new(name as *const u8, name_len)?;

                if let Some(obj) = self.overlays.read().get(&filename) {
                    let rights = AccessRights(
                        calling_rights.0 &
                            (AccessRights::MUTATE | AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE).0,
                    );
                    let handle_id = get_current_process()
                        .ok_or(InvocationError::InvalidHandle)?
                        .proc_handles
                        .write()
                        .insert(obj.clone(), rights);
                    return Ok(handle_id.0);
                }

                let underlying = self.underlying.read().clone();
                underlying.invoke(invocation, calling_rights).await
            },

            Invocation::Directory(DirectoryOp::Link { name, name_len, handle_id }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                let filename = Filename::new(name as *const u8, name_len)?;
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let obj_arc = {
                    let table = proc.proc_handles.read();
                    let entry = table.get(&handle_id).ok_or(InvocationError::InvalidHandle)?;
                    entry.object.clone()
                };

                self.overlays.write().insert(filename, obj_arc);
                Ok(0)
            },

            Invocation::Directory(DirectoryOp::Unlink { name, name_len }) => {
                if !calling_rights.contains(AccessRights::WRITE) {
                    return Err(InvocationError::AccessDenied);
                }
                let filename = Filename::new(name as *const u8, name_len)?.name;

                let removed = self.overlays.write().remove_entry(&*filename).is_some();
                if removed {
                    Ok(0)
                } else {
                    let underlying = self.underlying.read().clone();
                    underlying.invoke(invocation, calling_rights).await
                }
            },

            Invocation::Directory(DirectoryOp::List { offset, sink }) => {
                let proc = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let sink_obj = proc.proc_handles.read().resolve(sink, AccessRights::WRITE)?;

                let overlays_entries: Vec<(String, &'static str)> = {
                    let overlays = self.overlays.read();
                    overlays.iter().map(|(name, obj)| (name.name.to_string(), obj.type_name())).collect()
                };

                let underlying = self.underlying.read().clone();

                crate::core::asynchronous::Executor::new().spawn(async move {
                    // 1. Stream virtual overlays first
                    let mut iter = overlays_entries.iter().peekable();
                    while let Some((name_str, type_name)) = iter.next() {
                        let mut entry = AbiDirEntry {
                            entry_type: match *type_name {
                                "Directory" => DirEntryType::Directory as u8,
                                "File" => DirEntryType::File as u8,
                                _ => DirEntryType::Object as u8,
                            },
                            name_len: cmp::min(name_str.len(), 254) as u8,
                            name: [0u8; 254],
                        };
                        let len = entry.name_len as usize;
                        entry.name[..len].copy_from_slice(&name_str.as_bytes()[..len]);

                        let mut flags = PacketFlags::IS_STREAM;
                        // Always set HAS_NEXT because the underlying disk filesystem will stream next
                        flags = flags.insert(PacketFlags::HAS_NEXT);

                        let header = PacketHeader {
                            magic: VESPER_MAGIC,
                            version: 1,
                            packet_flags: flags,
                            packet_type: 1,
                            payload_len: size_of::<AbiDirEntry>() as u32,
                            reserved: 0,
                        };

                        let mut buffer = [0u8; size_of::<PacketHeader>() + size_of::<AbiDirEntry>()];
                        let header_size = size_of::<PacketHeader>();
                        let entry_size = size_of::<AbiDirEntry>();
                        unsafe {
                            let header_ptr = &header as *const _ as *const u8;
                            let entry_ptr = &entry as *const _ as *const u8;
                            copy_nonoverlapping(header_ptr, buffer.as_mut_ptr(), header_size);
                            copy_nonoverlapping(entry_ptr, buffer.as_mut_ptr().add(header_size), entry_size);
                        }

                        let op = FileOp::Write { offset: 0, buffer_ptr: buffer.as_mut_ptr() as usize, len: buffer.len() };
                        if sink_obj.invoke(Invocation::File(op), AccessRights::WRITE).await.is_err() {
                            return; // sink disconnected
                        }
                    }

                    // 2. Delegate to the underlying disk filesystem to stream all of its files/directories
                    let _ = underlying.invoke(Invocation::Directory(DirectoryOp::List { offset, sink }), calling_rights).await;
                });

                Ok(0)
            },

            Invocation::Directory(DirectoryOp::CreateFile { .. }) | Invocation::Directory(DirectoryOp::CreateDir { .. }) => {
                let underlying = self.underlying.read().clone();
                underlying.invoke(invocation, calling_rights).await
            },

            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
