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

        let dirty: Vec<Arc<Ext2File>> = {
            let files = fs.dirty_files.lock();
            files.values().cloned().collect()
        };

        for file in dirty {
            let _ = file.flush_dirty_pages().await;
        }

        let _ = fs.cache.flush().await;
    }
}

pub async fn mount_ext2_rootfs(raw_block_device: Arc<dyn AsyncBlockDevice>) -> Arc<Ext2Directory> {
    let mut gpt = GptTable::parse(raw_block_device).await.expect("Failed mounting GPT configuration table maps");

    let partition = Arc::new(gpt.partitions.remove(0));

    let ext2_fs = Arc::new(Ext2FileSystem::mount(partition).await.expect("Failed mounting Ext2 arch metadata"));

    // launch daemon after mounting 
    let root_inode_data = ext2_fs.read_inode(2).await.expect("Failed parsing root inode structs");
    let root_dir_object = Arc::new(Ext2Directory {
        fs: Arc::clone(&ext2_fs),
        inode_num: 2,
        inode_data: RwLock::new(root_inode_data),
    });

    #[cfg(feature = "diagnostics")]
    crate::tests::ext2_tests::test_ext2_owned_cache(&root_dir_object).await;

    Executor::new().spawn(ext2_writeback_daemon(ext2_fs));
    root_dir_object
}
