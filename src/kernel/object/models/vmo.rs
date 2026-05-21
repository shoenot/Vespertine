use alloc::sync::Arc;

use crate::{kernel::{object::{handle::AccessRights, invoke::{Invocation, InvocationError}, obj::KernelObject, op::VmoOp}, thread::get_current_process}, memory::vmo::{PagedBackingStore, Vmo}};


#[derive(Debug)]
pub struct VmoObject {
    vmo: Arc<dyn PagedBackingStore>,
}

impl KernelObject for VmoObject {
    fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        if let Invocation::Vmo(vmo_op) = invocation {
            match vmo_op {
                VmoOp::GetPage { offset } => { 
                    self.vmo.request_page(offset)
                        .map_err(|_| InvocationError::InvalidArgument)
                },
                VmoOp::Resize { new_size } => { 
                    self.vmo.resize_object(new_size)
                        .map_err(|_| InvocationError::UnsupportedOperation)?;
                    Ok(0)
                },
                VmoOp::Clone { offset, len } => { 
                    let child_vmo = self.vmo.clone_range(offset, len)
                        .map_err(|_| InvocationError::InvalidArgument)?;

                    let child_obj = Arc::new(VmoObject { vmo: child_vmo });

                    let current_proc = get_current_process()
                        .ok_or(InvocationError::UnsupportedOperation)?;

                    let handle_id = current_proc.proc_handles.write().insert(child_obj, AccessRights::all());

                    Ok(handle_id.0 as usize)
                },
            }
        } else {
            Err(InvocationError::UnsupportedOperation)
        }
    }

    fn type_name(&self) -> &'static str {
        "VMO"
    }
}
