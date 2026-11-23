use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::executor::Executor;
use crate::executor::async_sleep::sleep_async;
use crate::sync::RwLock;
use crate::storage::blockdev::AsyncBlockDevice;
use crate::storage::fs::ext2::Ext2FileSystem;
use crate::storage::fs::ext2::obj::{
    Ext2Directory,
    Ext2File,
};
use crate::storage::partition::gpt::GptTable;

async fn ext2_writeback_daemon(fs: Arc<Ext2FileSystem>) {
    loop {
        // sleep 5 seconds
        sleep_async(5000).await;

        fs.reap_pending().await;

        let dirty: Vec<Arc<Ext2File>> = {
            let files = fs.dirty_files.lock();
            files.values().cloned().collect()
        };

        for file in dirty {
            if let Err(error) = file.flush_dirty_pages().await {
                crate::klogln!("[EXT2] writeback failed for inode {}: {:?}", file.inode_num, error);
            }
        }

        if fs.cache.flush().await.is_err() {
            crate::klogln!("[EXT2] metadata cache flush failed");
        }
    }
}

pub async fn mount_ext2_rootfs(raw_block_device: Arc<dyn AsyncBlockDevice>) -> Arc<Ext2Directory> {
    let gpt = GptTable::parse(raw_block_device).await.expect("Failed to read the GPT partition table");
    let partition = gpt.partitions.into_iter().next().expect("No partition available for the ext2 root");
    let total_sectors = partition.total_sectors;
    let ext2_fs = Arc::new(
        Ext2FileSystem::mount(Arc::new(partition), total_sectors)
            .await.expect("Failed to mount the ext2 root"),
    );

    let root_inode_data = ext2_fs.read_inode(2).await.expect("Failed to read the ext2 root inode");
    let root_blocks = ext2_fs.directory_block_count(&root_inode_data).expect("Invalid ext2 root directory");
    assert!(root_blocks != 0 && root_inode_data.links_count >= 2, "Invalid ext2 root inode");
    let root_dir_object = Arc::new(Ext2Directory {
        fs: Arc::clone(&ext2_fs),
        inode_num: 2,
        inode_data: RwLock::new(root_inode_data),
    });

    ext2_fs.active_dirs.lock().insert(2, Arc::downgrade(&root_dir_object));

    #[cfg(feature = "diagnostics")]
    {
        crate::tests::ext2_tests::test_ext2_owned_cache(&root_dir_object).await;
        crate::tests::ext2_tests::test_ext2_unlink_and_truncate(&root_dir_object).await;
        test_ext2_sync(&root_dir_object, total_sectors).await;
    }

    Executor::new().spawn(ext2_writeback_daemon(ext2_fs));
    root_dir_object
}

#[cfg(feature = "diagnostics")]
async fn test_ext2_sync(root: &Ext2Directory, total_sectors: u64) {
    use alloc::vec;
    use core::sync::atomic::Ordering::SeqCst;
    use vespertine_abi::{AccessRights, FileOp, Invocation, UserID};

    use crate::drivers::virtio::blk::{DIAGNOSTIC_FAIL_FLUSH, DIAGNOSTIC_FLUSH_COUNT};
    use crate::memory::PAGE_SIZE_4K;
    use crate::object::invoke::InvocationError;
    use crate::object::obj::KernelDirectory;

    const NAME: &str = ".ext2-sync-test";

    let parent = *root.inode_data.read();
    if root.fs.lookup_in_dir(&parent, NAME).await.unwrap().is_some() {
        root.unlink_child(NAME).await.unwrap();
        root.fs.reap_pending().await;
    }

    let file = root.create_child_file(NAME, UserID(0)).await.unwrap();
    let payload: Vec<u8> = (0..PAGE_SIZE_4K + 73)
        .map(|i| ((i * 31) ^ (i >> 8)) as u8)
        .collect();

    let write = Invocation::File(FileOp::Write {
        offset: 0,
        buffer_ptr: payload.as_ptr() as usize,
        len: payload.len(),
    });
    assert_eq!(file.invoke(write, AccessRights::WRITE).await.unwrap(), payload.len());

    let sync = || Invocation::File(FileOp::Sync);
    let completed = DIAGNOSTIC_FLUSH_COUNT.load(SeqCst);
    assert_eq!(file.invoke(sync(), AccessRights::READ).await, Err(InvocationError::AccessDenied));

    // Before the daemon starts, simulate an allocator with no free blocks.
    let counts: Vec<u16> = {
        let mut groups = root.fs.bgdt.lock();
        groups.iter_mut().map(|group| {
            let saved = group.free_blocks_count;
            group.free_blocks_count = 0;
            saved
        }).collect()
    };
    let no_space = file.invoke(sync(), AccessRights::WRITE).await;
    {
        let mut groups = root.fs.bgdt.lock();
        for (group, count) in groups.iter_mut().zip(counts) {
            group.free_blocks_count = count;
        }
    }
    assert_eq!(no_space, Err(InvocationError::NoSpace));
    assert_eq!(DIAGNOSTIC_FLUSH_COUNT.load(SeqCst), completed);

    DIAGNOSTIC_FAIL_FLUSH.store(true, SeqCst);
    assert_eq!(file.invoke(sync(), AccessRights::WRITE).await, Err(InvocationError::Io));
    assert!(!DIAGNOSTIC_FAIL_FLUSH.load(SeqCst), "sync never submitted a device flush");
    assert_eq!(DIAGNOSTIC_FLUSH_COUNT.load(SeqCst), completed);

    file.invoke(sync(), AccessRights::WRITE).await.unwrap();
    assert_eq!(DIAGNOSTIC_FLUSH_COUNT.load(SeqCst), completed + 1);

    // Read with independent inode, metadata, and file caches.
    let cold_fs = Arc::new(
        Ext2FileSystem::mount(root.fs.partition.clone(), total_sectors).await.unwrap(),
    );
    let cold_root = Ext2Directory {
        inode_num: 2,
        inode_data: RwLock::new(cold_fs.read_inode(2).await.unwrap()),
        fs: cold_fs,
    };
    let cold_file = cold_root.lookup_child(NAME).await.unwrap();
    let mut actual = vec![0; payload.len() + 1];
    let read = Invocation::File(FileOp::Read {
        offset: 0,
        buffer_ptr: actual.as_mut_ptr() as usize,
        len: actual.len(),
    });
    assert_eq!(cold_file.invoke(read, AccessRights::READ).await.unwrap(), payload.len());
    assert_eq!(&actual[..payload.len()], &payload);

    drop((cold_file, cold_root, file));
    root.unlink_child(NAME).await.unwrap();
    root.fs.reap_pending().await;
    root.fs.cache.flush().await.unwrap();

    crate::klogln!("[TEST] ext2 sync device completion, cold read, and error propagation passed");
}
