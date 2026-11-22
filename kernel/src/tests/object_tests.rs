use vespertine_abi::op::MemPoolOp;
use vespertine_abi::{
    AccessRights, FileOp, HandleID, Invocation
};

use crate::object::vfs::{
    kernel_invoke,
    kernel_walk,
};
use crate::klogln;

const TEST_ELF: &str = "/System/Core/hesper";

pub(super) async fn run() {
    test_native_vmo_rights().await;
    test_pool_reservation_lifetime().await;
    test_private_copy_pool_charge().await;
    test_failed_spawn_releases_child().await;
}

async fn test_native_vmo_rights() {
    use alloc::sync::Arc;
    use vespertine_abi::{ProcOp, VmoOp};

    use crate::memory::{FaultError, PAGE_SIZE_4K as PAGE};
    use crate::memory::vmo::AnonymousVmo;
    use crate::object::fs::file::FileObj;
    use crate::object::invoke::InvocationError::{AccessDenied, UnsupportedOperation};
    use crate::object::vfs::{kernel_close, kernel_register_obj};
    use crate::object::vmo::VmoObject;
    use crate::process::current_process;

    const USER: usize = 1 << 2;
    const RW: usize = USER | (1 << 0);
    const RX: usize = USER | (1 << 1);
    const NONE: usize = USER | (1 << 7);
    const SELF: HandleID = HandleID(1);

    let map = |vm_flags| Invocation::Vmo(VmoOp::MapIntoProc {
        vaddr: 0,
        len: PAGE,
        vm_flags,
    });
    let protect = |vaddr, prot| Invocation::Proc(ProcOp::Mprotect {
        vaddr,
        len: PAGE,
        prot,
    });
    let unmap = |vaddr| Invocation::Proc(ProcOp::Unmap { vaddr, len: PAGE });

    // Ext2 gives us a readable cache handle with mapping disabled.
    let file = kernel_walk(TEST_ELF, HandleID(0), AccessRights::READ)
        .await.unwrap();
    let raw = HandleID(kernel_invoke(file, Invocation::File(FileOp::GetVmo))
        .await.unwrap());
    assert_eq!(kernel_invoke(raw, map(USER)).await, Err(AccessDenied));

    for offset in [0, PAGE] {
        let private_copy = || Invocation::File(FileOp::GetPrivateVmo { offset, len: PAGE });
        let get_page = || Invocation::Vmo(VmoOp::GetPage { offset: 0 });

        let first = HandleID(kernel_invoke(file, private_copy()).await.unwrap());
        let second = HandleID(kernel_invoke(file, private_copy()).await.unwrap());

        let addr = kernel_invoke(first, map(RW)).await.unwrap();
        assert_eq!(kernel_invoke(first, map(RX)).await, Err(AccessDenied));

        let first_phys = kernel_invoke(first, get_page()).await.unwrap();
        let second_phys = kernel_invoke(second, get_page()).await.unwrap();
        assert_ne!(first_phys, second_phys);

        let direct_map = *crate::memory::DIRECT_MAP_OFFSET;
        let first_ptr = (first_phys + direct_map) as *mut u8;
        let second_ptr = (second_phys + direct_map) as *const u8;
        let original = unsafe { second_ptr.read() };

        assert_eq!(unsafe { first_ptr.read() }, original);
        unsafe { first_ptr.write(original ^ 0xff) };
        assert_eq!(unsafe { second_ptr.read() }, original);

        let fresh = HandleID(kernel_invoke(file, private_copy()).await.unwrap());
        let fresh_phys = kernel_invoke(fresh, get_page()).await.unwrap();
        assert_eq!(unsafe { ((fresh_phys + direct_map) as *const u8).read() }, original);

        kernel_invoke(SELF, unmap(addr)).await.unwrap();
        for handle in [first, second, fresh] {
            kernel_close(handle).unwrap();
        }
    }

    kernel_close(raw).unwrap();
    kernel_close(file).unwrap();

    // The in-memory file produces a mappable snapshot with attenuated rights.
    let bytes = b"read-only snapshot";
    let file = kernel_register_obj(
        Arc::new(FileObj::new(bytes.as_ptr(), bytes.len())),
        AccessRights::READ,
    );
    let snapshot = HandleID(kernel_invoke(file, Invocation::File(FileOp::GetVmo))
        .await.unwrap());
    kernel_close(file).unwrap();

    assert_eq!(kernel_invoke(snapshot, map(RW)).await, Err(AccessDenied));
    assert_eq!(kernel_invoke(snapshot, map(RX)).await, Err(AccessDenied));

    let resize = Invocation::Vmo(VmoOp::Resize { new_size: 0 });
    assert_eq!(kernel_invoke(snapshot, resize).await, Err(UnsupportedOperation));

    let clone = Invocation::Vmo(VmoOp::Clone { offset: 0, len: PAGE });
    let child = HandleID(kernel_invoke(snapshot, clone).await.unwrap());
    kernel_close(snapshot).unwrap();

    assert_eq!(kernel_invoke(child, map(RW)).await, Err(AccessDenied));
    assert_eq!(kernel_invoke(child, map(RX)).await, Err(AccessDenied));

    let addr = kernel_invoke(child, map(USER)).await.unwrap();
    kernel_close(child).unwrap();
    assert_eq!(kernel_invoke(SELF, protect(addr, RW)).await, Err(AccessDenied));
    assert_eq!(kernel_invoke(SELF, protect(addr, RX)).await, Err(AccessDenied));
    kernel_invoke(SELF, unmap(addr)).await.unwrap();

    // A backed no-access mapping keeps the RW ceiling after handle closure.
    let backing = AnonymousVmo::new(PAGE);
    let frame = backing.get_page(0).unwrap();
    let vmo = kernel_register_obj(
        Arc::new(VmoObject::new(backing)),
        AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE,
    );
    let addr = kernel_invoke(vmo, map(NONE)).await.unwrap();
    kernel_close(vmo).unwrap();

    let proc = current_process().unwrap();
    let fault = || { 
        let slot = proc.vmm.read();
        slot.as_ref()
            .expect("test process has no address space")
            .handle_page_fault(addr, USER)
    };
    assert_eq!(fault(), Err(FaultError::AccessDenied));

    kernel_invoke(SELF, protect(addr, RW)).await.unwrap();
    assert_eq!(fault(), Ok(()));
    assert_eq!(Arc::strong_count(&frame), 3);
    unsafe { (frame.virt() as *mut u8).write(0x5a) };

    kernel_invoke(SELF, protect(addr, NONE)).await.unwrap();
    assert_eq!(Arc::strong_count(&frame), 2);
    assert_eq!(fault(), Err(FaultError::AccessDenied));

    kernel_invoke(SELF, protect(addr, RW)).await.unwrap();
    assert_eq!(fault(), Ok(()));
    assert_eq!(unsafe { (frame.virt() as *const u8).read() }, 0x5a);

    kernel_invoke(SELF, unmap(addr)).await.unwrap();
    assert_eq!(Arc::strong_count(&frame), 1);

    klogln!("  Native VMO rights and 4 KiB protection passed");
}

async fn test_pool_reservation_lifetime() {
    use alloc::sync::Arc;
    use vespertine_abi::{ProcOp, VmoOp};

    use crate::memory::PAGE_SIZE_4K as PAGE;
    use crate::object::invoke::InvocationError;
    use crate::object::mempool::MemPool;
    use crate::object::vfs::{kernel_close, kernel_register_obj};
    use crate::process::current_process;

    let parent = kernel_register_obj(
        Arc::new(MemPool::new(Some(PAGE), None)),
        AccessRights::WRITE | AccessRights::CREATE,
    );
    let new_subpool = || Invocation::MemPool(MemPoolOp::CreateSubPool { limit: PAGE });
    let left = HandleID(kernel_invoke(parent, new_subpool()).await.unwrap());
    let right = HandleID(kernel_invoke(parent, new_subpool()).await.unwrap());

    let allocate = || Invocation::MemPool(MemPoolOp::AllocateVmo { size: 1 });
    let vmo = HandleID(kernel_invoke(left, allocate()).await.unwrap());

    let map = Invocation::Vmo(VmoOp::MapIntoProc {
        vaddr: 0,
        len: 1,
        vm_flags: 0b101, // USER | WRITE
    });
    let addr = kernel_invoke(vmo, map).await.unwrap();

    // Install a writable PTE so unmap also goes through frame retirement.
    {
        let slot = current_process().unwrap().vmm.read();
        slot.as_ref()
            .expect("test process has no address space")
            .handle_page_fault(addr, 0b110)
            .unwrap();
    }

    kernel_close(vmo).unwrap();
    assert_eq!(
        kernel_invoke(right, allocate()).await,
        Err(InvocationError::PoolExhausted),
        "the live mapping still owns the parent's page reservation",
    );

    let unmap = Invocation::Proc(ProcOp::Unmap { vaddr: addr, len: PAGE });
    kernel_invoke(HandleID(1), unmap).await.unwrap();

    let recycled = HandleID(kernel_invoke(right, allocate()).await.unwrap());
    kernel_close(recycled).unwrap();

    // Check that the original leaf pool got its quota back too.
    let recycled = HandleID(kernel_invoke(left, allocate()).await.unwrap());
    kernel_close(recycled).unwrap();

    for handle in [left, right, parent] {
        kernel_close(handle).unwrap();
    }

    klogln!("[TEST] Mempool reservation follows VMO backing");
}

async fn test_private_copy_pool_charge() {
    use vespertine_abi::{ProcOp, VmoOp};

    use crate::memory::PAGE_SIZE_4K as PAGE;
    use crate::object::invoke::InvocationError;
    use crate::object::vfs::kernel_close;
    use crate::process::{DEFAULT_PROCESS_MEMORY_LIMIT, current_process};

    let proc = current_process().unwrap();
    let _reservation = proc.default_pool
        .allocate_backing(DEFAULT_PROCESS_MEMORY_LIMIT - PAGE)
        .expect("leave one page available in the kernel test pool");

    let file = kernel_walk(TEST_ELF, HandleID(0), AccessRights::READ)
        .await.unwrap();

    let copy = |len| Invocation::File(FileOp::GetPrivateVmo { offset: 0, len });
    let allocate = || Invocation::MemPool(MemPoolOp::AllocateVmo { size: 1 });

    assert_eq!(
        kernel_invoke(file, copy(PAGE + 1)).await,
        Err(InvocationError::PoolExhausted),
        "a two-page copy must fail with one page of quota left",
    );

    let private = HandleID(kernel_invoke(file, copy(1)).await.unwrap());
    let addr = kernel_invoke(
        private,
        Invocation::Vmo(VmoOp::MapIntoProc {
            vaddr: 0,
            len: 1,
            vm_flags: 0b101, // USER | WRITE
        }),
    ).await.unwrap();
    kernel_close(private).unwrap();

    assert_eq!(
        kernel_invoke(HandleID(4), allocate()).await,
        Err(InvocationError::PoolExhausted),
        "the private mapping must still hold the default pool's last page",
    );

    kernel_invoke(
        HandleID(1),
        Invocation::Proc(ProcOp::Unmap { vaddr: addr, len: PAGE }),
    ).await.unwrap();

    let recycled = HandleID(kernel_invoke(HandleID(4), allocate()).await.unwrap());
    kernel_close(recycled).unwrap();
    kernel_close(file).unwrap();

    klogln!("[TEST] Private file copy shares the default pool quota");
}

async fn test_failed_spawn_releases_child() {
    use alloc::sync::Arc;
    use vespertine_abi::{ProcManOp, SpawnCredentials};

    use crate::memory::PAGE_SIZE_4K as PAGE;
    use crate::object::fs::directory::Directory;
    use crate::object::invoke::InvocationError;
    use crate::object::vfs::{kernel_close, kernel_register_obj};
    use crate::process::procman::ProcessManager;

    let pm = kernel_register_obj(Arc::new(ProcessManager {}), AccessRights::CREATE);
    let exec = kernel_walk(
        TEST_ELF,
        HandleID(0),
        AccessRights::READ | AccessRights::EXECUTE,
    ).await.expect("open valid ELF for rollback test");

    let marker = Arc::new(Directory::new());
    let marker_handle = kernel_register_obj(marker.clone(), AccessRights::TRAVERSE);
    let baseline = Arc::strong_count(&marker);

    let mut args = alloc::vec![b'x'; PAGE];
    args[PAGE - 1] = 0;
    let name = "failed-spawn-rollback";

    let op = ProcManOp::Spawn {
        name_ptr: name.as_ptr() as usize,
        name_len: name.len(),
        exec_handle: exec,
        root_handle: marker_handle,
        root_rights: AccessRights::TRAVERSE,
        cwd_handle: marker_handle,
        cwd_rights: AccessRights::TRAVERSE,
        source: HandleID(usize::MAX),
        sink: HandleID(usize::MAX),
        credentials: SpawnCredentials::Inherit,
        capabilities_ptr: 0,
        capabilities_len: 0,
        args_buffer_ptr: args.as_ptr() as usize,
        args_buffer_len: args.len(),
        start_suspended: true,
    };

    assert_eq!(
        kernel_invoke(pm, Invocation::ProcessManager(op)).await,
        Err(InvocationError::OutOfMemory),
        "spawn must fail during initial stack setup",
    );
    assert_eq!(
        Arc::strong_count(&marker),
        baseline,
        "failed spawn retained its child handle table",
    );

    for handle in [marker_handle, exec, pm] {
        kernel_close(handle).unwrap();
    }

    klogln!("[TEST] Failed spawn releases child");
}
