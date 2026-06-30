use vespertine_abi::op::{
    ChannelOp,
    MemManOp,
    MemPoolOp,
};
use vespertine_abi::{
    AccessRights,
    HandleID,
    Invocation,
};

use crate::core::object::ipc::socket::init_ipc_pipeline;
use crate::core::object::vfs::{
    kernel_invoke,
    kernel_walk,
};
use crate::klogln;

pub async fn run_pool_tests() {
    let mm_handle = kernel_walk("/Objects/MemoryManager", HandleID(0), AccessRights::all()).await.expect("No Memory Manager found");

    let root_pool_handle = HandleID(
        kernel_invoke(mm_handle, Invocation::MemoryManager(MemManOp::CreatePool { limit: 0 })).await.expect("Failed to create root pool"),
    );
    klogln!("  - Created global root pool: {:?}", root_pool_handle);

    let sub_pool_handle = HandleID(
        kernel_invoke(root_pool_handle, Invocation::MemPool(MemPoolOp::CreateSubPool { limit: 1024 * 1024 }))
            .await
            .expect("Failed to create sub pool"),
    );
    klogln!("  - Created 1mb sub pool: {:?}", sub_pool_handle);

    let vmo_handle = HandleID(
        kernel_invoke(sub_pool_handle, Invocation::MemPool(MemPoolOp::AllocateVmo { size: 4096 })).await.expect("Failed to allocate VMO"),
    );
    klogln!("  - Allocated 4kb vmo: {:?}", vmo_handle);

    let break_attempt = kernel_invoke(sub_pool_handle, Invocation::MemPool(MemPoolOp::AllocateVmo { size: 1024 * 2048 })).await;
    klogln!("  - Attempted overflow allocation result: {:?}", break_attempt);
}

pub async fn run_channel_ipc_tests() {
    klogln!("  - Initializing kernel IPC pipeline...");
    let (tx, rx) = init_ipc_pipeline();

    // push a small message into the tx channel handle
    let mut data = [0u8; 64];
    data[..12].copy_from_slice(b"Hello Kernel");
    let push_op = ChannelOp::PushSmall { data, len: 12 };

    kernel_invoke(tx, Invocation::Channel(push_op)).await.expect("Failed to push to channel");

    // pull the message from the rx channel handle
    let mut rx_buf = [0u8; 64];
    let pull_op = ChannelOp::Pull { buffer_ptr: rx_buf.as_mut_ptr() as usize };

    let bytes_pulled = kernel_invoke(rx, Invocation::Channel(pull_op)).await.expect("Failed to pull from channel");

    assert_eq!(bytes_pulled, 12);
    assert_eq!(&rx_buf[..12], b"Hello Kernel");
    klogln!("  - Channel loopback push/pull verified successfully!");
}

pub async fn run_object_tests() {
    klogln!("Running Post-VFS Object and Memory Manager Tests...");
    run_pool_tests().await;

    klogln!("Running Post-VFS Kernel IPC Tests...");
    run_channel_ipc_tests().await;

    klogln!("All Post-VFS tests passed!");
}
