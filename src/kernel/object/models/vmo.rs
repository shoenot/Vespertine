use alloc::sync::Arc;

use crate::{kernel::object::{invoke::{Invocation, InvocationError}, obj::KernelObject, op::VmoOp}, memory::vmo::{PagedBackingStore, Vmo}};


#[derive(Debug)]
pub struct VmoObject {
    vmo: Arc<dyn PagedBackingStore>,
}

impl KernelObject for VmoObject {
    fn invoke(&self, invocation: Invocation) -> Result<usize, InvocationError> {
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
                    Err(InvocationError::UnsupportedOperation)
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
