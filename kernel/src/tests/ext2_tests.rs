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

    // Write back the retained block's zeroed tail before the cold reopen.
    file.flush_dirty_pages().await.expect("flush truncated tail");
    root.fs.cache.flush().await.expect("flush resized inode");
    let weak_file = Arc::downgrade(&file);
    drop(file);
    drop(description);
    assert!(weak_file.upgrade().is_none(), "fixture cache stayed alive");

    let cold = root.lookup_child(NAME).await.expect("reopen fixture cold");
    read_exact(cold.as_ref(), &mut actual).await;
    assert_eq!(actual, expected, "cold shrink/re-extension failed");

    cold.invoke(truncate(0), AccessRights::WRITE).await.unwrap();
    root.fs.reap_pending().await;
    root.fs.cache.flush().await.expect("flush fixture cleanup");
}

pub(crate) async fn test_ext2_unlink_and_truncate(root: &Ext2Directory) {
    use alloc::vec;
    use vespertine_abi::{AccessRights, FileOp, Invocation, UserID};

    use crate::memory::vmo::PagedBackingStore;
    use crate::object::invoke::InvocationError;
    use crate::object::obj::KernelDirectory;

    const NAME: &str = ".ext2-reclaim-test";
    const REPLACEMENT: &str = ".ext2-reclaim-replacement";

    let fs = &root.fs;
    let inode_for = async |name| {
        let parent = *root.inode_data.read();
        fs.lookup_in_dir(&parent, name).await.unwrap()
    };
    let free_blocks = || {
        fs.bgdt.lock().iter().map(|group| u32::from(group.free_blocks_count)).sum::<u32>()
    };
    let free_inodes = || {
        fs.bgdt.lock().iter().map(|group| u32::from(group.free_inodes_count)).sum::<u32>()
    };

    for name in [NAME, REPLACEMENT] {
        if inode_for(name).await.is_some() {
            root.unlink_child(name).await.expect("remove prior diagnostic fixture");
        }
    }
    fs.reap_pending().await;

    let description = root.create_child_file(NAME, UserID(0)).await.unwrap();
    let inode = inode_for(NAME).await.unwrap();
    let file = fs.active_files.lock().get(&inode).unwrap().upgrade().unwrap();

    let block_size = fs.block_size as usize;
    let old_size = 15 * block_size + 37;
    let keep = 12 * block_size + 137;
    let payload = vec![0x75; old_size];
    let write = Invocation::File(FileOp::Write {
        offset: 0, buffer_ptr: payload.as_ptr() as usize, len: old_size,
    });
    assert_eq!(description.invoke(write, AccessRights::WRITE).await.unwrap(), old_size);
    file.flush_dirty_pages().await.unwrap();

    let before = free_blocks();
    let truncate = |size| Invocation::File(FileOp::Truncate { size });
    description.invoke(truncate(keep), AccessRights::WRITE).await.unwrap();
    assert_eq!(free_blocks(), before, "truncate released blocks before commit flush");
    fs.reap_pending().await;
    assert_eq!(free_blocks(), before + 3, "single-indirect shrink lost allocation accounting");

    description.invoke(truncate(old_size), AccessRights::WRITE).await.unwrap();
    file.flush_dirty_pages().await.unwrap();
    let weak = Arc::downgrade(&file);
    drop((file, description));
    assert!(weak.upgrade().is_none(), "reopen must be cold");

    let description = root.lookup_child(NAME).await.unwrap();
    let file = fs.active_files.lock().get(&inode).unwrap().upgrade().unwrap();
    let mut output = vec![0xff; old_size];
    let read = Invocation::File(FileOp::Read {
        offset: 0, buffer_ptr: output.as_mut_ptr() as usize, len: old_size,
    });
    assert_eq!(description.invoke(read, AccessRights::READ).await.unwrap(), old_size);
    assert_eq!(&output[..keep], &payload[..keep]);
    assert!(output[keep..].iter().all(|&byte| byte == 0), "cold regrowth exposed old bytes");

    let live_backing: Arc<dyn PagedBackingStore> = file.clone();
    let allocated = file.inode_data.read().blocks / fs.sectors_per_block;
    root.unlink_child(NAME).await.unwrap();
    assert!(matches!(root.lookup_child(NAME).await, Err(InvocationError::PathNotFound)));
    let mut byte = 0u8;
    let read = Invocation::File(FileOp::Read {
        offset: 0, buffer_ptr: &mut byte as *mut u8 as usize, len: 1,
    });
    assert_eq!(description.invoke(read, AccessRights::READ).await.unwrap(), 1);
    assert_eq!(byte, payload[0]);
    let replacement_byte = 0x35u8;
    let write = Invocation::File(FileOp::Write {
        offset: keep, buffer_ptr: &replacement_byte as *const u8 as usize, len: 1,
    });
    assert_eq!(description.invoke(write, AccessRights::WRITE).await.unwrap(), 1);

    let replacement = root.create_child_file(REPLACEMENT, UserID(0)).await.unwrap();
    assert_ne!(inode_for(REPLACEMENT).await, Some(inode), "reused an inode while it was open");
    let before_inodes = free_inodes();
    let before_blocks = free_blocks();

    drop((file, description));
    fs.reap_pending().await;
    assert_eq!(free_inodes(), before_inodes, "reaped an inode held by its live backing");

    drop(live_backing);
    fs.reap_pending().await;
    assert_eq!(free_inodes(), before_inodes + 1);
    assert_eq!(free_blocks(), before_blocks + allocated, "unlink leaked allocated pointers");

    drop(replacement);
    root.unlink_child(REPLACEMENT).await.unwrap();
    fs.reap_pending().await;
    fs.cache.flush().await.unwrap();
}
