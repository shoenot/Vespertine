use alloc::boxed::Box;
use alloc::sync::Arc;
use hal::mmu::PageSize;
use core::any::Any;

use async_trait::async_trait;
use vespertine_abi::op::VmoOp;
use vespertine_abi::{
    AccessRights,
    Invocation,
};

use crate::memory::vmm::{CachePolicy, MapBehavior, VmOptions, VmPermissions, VmaBacking, VmaChargeKind};
use crate::object::help::RightsWrapper;
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::object::obj::KernelObject;
use crate::process::current_process;
use crate::memory::vmo::{AnonymousVmo, PagedBackingStore};

#[derive(Debug)]
pub struct VmoObject {
    pub vmo: Arc<dyn PagedBackingStore>,
}

impl VmoObject {
    pub fn new(vmo: Arc<dyn PagedBackingStore>) -> Self { Self { vmo } }
}

#[async_trait]
impl KernelObject for VmoObject {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights, _ctx: InvokeContext<'_>) -> Result<usize, InvocationError> {
        if let Invocation::Vmo(vmo_op) = invocation {
            match vmo_op {
                VmoOp::GetPage { offset } => self.vmo.request_page(offset).map_err(|_| InvocationError::InvalidArgument),
                VmoOp::Resize { new_size } => { Err(InvocationError::UnsupportedOperation) },
                VmoOp::Clone { offset, len } => {
                    calling_rights.err_if_no(AccessRights::READ)?;

                    let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
                    let backing = self.vmo.clone_range(offset, len, proc.default_pool.state())?;
                    let handle = proc.handles.write().insert(Arc::new(VmoObject::new(backing)), calling_rights);
                    Ok(handle.0)
                },
                VmoOp::MapIntoProc { vaddr, len, vm_flags } => {
                    calling_rights.err_if_no(AccessRights::READ)?;

                    let permissions = VmPermissions::from_user_vm_flags(vm_flags)?;
                    let device_len = self.vmo.device_mapping_len();
                    if device_len.is_some_and(|limit| len > limit) {
                        return Err(InvocationError::InvalidArgument);
                    }

                    let mut max_permissions = VmPermissions::USER;
                    if calling_rights.contains(AccessRights::WRITE) {
                        max_permissions = max_permissions | VmPermissions::WRITE;
                    }
                    if device_len.is_none() && calling_rights.contains(AccessRights::EXECUTE) {
                        max_permissions = max_permissions | VmPermissions::EXECUTE;
                    }

                    let (cache, charge) = if device_len.is_some() {
                        (CachePolicy::Device, VmaChargeKind::Device)
                    } else {
                        (CachePolicy::Normal, VmaChargeKind::Private)
                    };

                    let opts = VmOptions {
                        permissions,
                        max_permissions,
                        cache,
                        page_size: PageSize::Size4K,
                        charge,
                    };

                    let proc = current_process().ok_or(InvocationError::UnsupportedOperation)?;
                    let mut slot = proc.vmm.write();
                    let vmm = slot.as_mut().ok_or(InvocationError::InvalidHandle)?;
                    let backing = VmaBacking::Vmo(self.vmo.clone());

                    let mapped = if vaddr == 0 {
                        vmm.reserve(len, opts, backing)
                    } else {
                        vmm.map_at(vaddr, len, opts, backing, 0, MapBehavior::ReplaceContained)
                    };

                    mapped.map_err(InvocationError::from)
                },
            }
        } else {
            Err(InvocationError::UnsupportedOperation)
        }
    }

    fn type_name(&self) -> &'static str { "VMO" }

    fn as_any(&self) -> &dyn Any { self }
}

pub fn new_private_file_vmo(offset: usize, len: usize, file_size: usize) -> Result<Arc<AnonymousVmo>, InvocationError> {
    let page = PageSize::Size4K.bytes();

    if len == 0 || offset % page != 0 || offset >= file_size {
        return Err(InvocationError::InvalidArgument);
    }
    if len.div_ceil(page) > (file_size - offset).div_ceil(page) {
        return Err(InvocationError::InvalidArgument);
    }

    let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
    proc.default_pool.allocate_backing(len)
}

pub fn install_private_file_vmo(backing: Arc<AnonymousVmo>, file_rights: AccessRights) -> Result<usize, InvocationError> {
    let rights = AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE | (file_rights & AccessRights::EXECUTE);

    let proc = current_process().ok_or(InvocationError::UnsupportedOperation)?;
    let handle = proc.handles.write().insert(Arc::new(VmoObject::new(backing)), rights);
    Ok(handle.0)
}
