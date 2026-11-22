use alloc::sync::Arc;

use crate::storage::fs::Ext2Directory;

pub(crate) async fn test_ext2_owned_cache(root: &Ext2Directory) {
    use alloc::vec;
    use vespertine_abi::{AccessRights, FileOp, Invocation};

    use crate::memory::{PAGE_SIZE_4K as PAGE, PhysBuffer};
    use crate::object::obj::{KernelDirectory, KernelObject};
    use crate::storage::fs::VfsNode;

    async fn read_exact(file: &dyn KernelObject, output: &mut [u8]) {
        let len = output.len();
        let bytes = file.invoke(
            Invocation::File(FileOp::Read {
                offset: 0,
                buffer_ptr: output.as_mut_ptr() as usize,
                len,
            }),
            AccessRights::READ,
        ).await.expect("read Ext2 test fixture");

        assert_eq!(bytes, len, "short read from Ext2 test fixture");
    }

    const NAME: &str = ".ext2-cache-test";
    const GAP: usize = 13;
    const KEEP: usize = 137;
    const PATTERN: u8 = 0xa7;

    let description = root.lookup_child(NAME).await.expect("open Ext2 test fixture");
    let root_inode = *root.inode_data.read();
    let inode = root.fs.lookup_in_dir(&root_inode, NAME)
        .await.expect("look up fixture inode")
        .expect("fixture inode missing");
    let file = {
        let active = root.fs.active_files.lock();
        active.get(&inode).and_then(|weak| weak.upgrade())
            .expect("fixture missing from active files")
    };

    let truncate = |size| Invocation::File(FileOp::Truncate { size });
    description.invoke(truncate(0), AccessRights::WRITE).await.unwrap();

    let data = vec![PATTERN; PAGE + 37];
    let end = GAP + data.len();
    let write = Invocation::File(FileOp::Write {
        offset: GAP,
        buffer_ptr: data.as_ptr() as usize,
        len: data.len(),
    });
    assert_eq!(
        description.invoke(write, AccessRights::WRITE).await.unwrap(),
        data.len(),
    );

    let mut expected = vec![0; end];
    expected[GAP..].copy_from_slice(&data);
    let mut actual = vec![0; end];
    read_exact(description.as_ref(), &mut actual).await;
    assert_eq!(actual, expected, "write past EOF or warm read failed");

    file.flush_dirty_pages().await.expect("flush fixture data");
    root.fs.cache.flush().await.expect("flush fixture metadata");

    // Read the partial second page straight from disk.
    {
        let _guard = file.file_vmo.io_lock.lock().await;
        let disk_page = PhysBuffer::new().expect("allocate disk-read buffer");
        let len = end - PAGE;
        let bytes = file.read_at_phys(PAGE, disk_page.phys(), len)
            .await.expect("read partial page from disk");
        assert_eq!(bytes, len);

        let disk_bytes = unsafe {
            core::slice::from_raw_parts(disk_page.virt() as *const u8, len)
        };
        assert_eq!(disk_bytes, &expected[PAGE..], "partial-page writeback failed");
    }

    // Hold an extra reference while truncate removes the cache entry.
    let held = file.file_vmo.cached_frame(PAGE).expect("second page is cached");
    let weak_frame = Arc::downgrade(&held);

    description.invoke(truncate(KEEP), AccessRights::WRITE).await.unwrap();
    assert!(file.file_vmo.cached_frame(PAGE).is_none());
    assert!(weak_frame.upgrade().is_some(), "truncate reclaimed a held frame");
    drop(held);
    assert!(weak_frame.upgrade().is_none(), "discarded frame still has an owner");

    description.invoke(truncate(end), AccessRights::WRITE).await.unwrap();
    expected[KEEP..].fill(0);

    read_exact(description.as_ref(), &mut actual).await;
    assert_eq!(actual, expected, "warm shrink/re-extension failed");

    // Drop the original file object so the reopen has a fresh page cache.
    root.fs.cache.flush().await.expect("flush resized inode");
    let weak_file = Arc::downgrade(&file);
    drop(file);
    drop(description);
    assert!(weak_file.upgrade().is_none(), "fixture cache stayed alive");

    let cold = root.lookup_child(NAME).await.expect("reopen fixture cold");
    read_exact(cold.as_ref(), &mut actual).await;
    assert_eq!(actual, expected, "cold shrink/re-extension failed");

    cold.invoke(truncate(0), AccessRights::WRITE).await.unwrap();
    root.fs.cache.flush().await.expect("flush fixture cleanup");

    crate::klogln!("[TEST] Ext2 owned cache, truncate, and writeback passed");
}
