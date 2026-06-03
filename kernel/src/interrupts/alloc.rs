use alloc::{collections::btree_map::BTreeMap, vec::Vec};
use crate::core::sync::TicketLock;
use crate::klogln;

use crate::core::sync::KernelOnceCell;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsiHandle {
    id: usize, 
}

impl MsiHandle {
    pub fn id(&self) -> usize { self.id }
}

struct VectorMeta {
    owner: usize,
    vector: u8,
    affinity_mask: usize,
}

struct AllocState {
    next_handle: usize,
    next_vector: u8,
    handles: BTreeMap<usize, Vec<VectorMeta>>,
}
 
static ALLOC_STATE: TicketLock<AllocState> = TicketLock::new(AllocState {
    next_handle: 1,
    next_vector: 0x40,
    handles: BTreeMap::new(),
});

pub struct ArchMsiFns {
    pub program_msi: fn(vector: u8, target_apic_id: u32, data: u32),
    pub free_vector: fn(vector: u8),
    pub register_irq_entry: extern "C" fn(vector: u8, handler: extern "C" fn(arg: usize), arg: usize),
}

static ARCH_FUNCS: KernelOnceCell<ArchMsiFns> = KernelOnceCell::new();

pub fn init_arch(funcs: ArchMsiFns) {
    ARCH_FUNCS.get_or_init(|| funcs);
}

pub fn msi_allocate(n: usize, _preferred_mask: usize) -> Result<MsiHandle, ()> {
    let mut st = ALLOC_STATE.lock();
    let hid = st.next_handle;
    st.next_handle += 1;
    let mut vecs = Vec::new();
    let mut vec_list = Vec::new();
    for _ in 0..n {
        let v = st.next_vector;
        st.next_vector = st.next_vector.wrapping_add(1);
        vecs.push(VectorMeta { owner: hid, vector: v, affinity_mask: 0 });
        vec_list.push(v);
    }
    st.handles.insert(hid, vecs);
    klogln!("[MSI] msi_allocate: handle={} vectors={:?}", hid, vec_list);
    Ok(MsiHandle { id: hid })
}

pub fn msi_register(
    handle: &MsiHandle, 
    idx: usize, 
    bus: u8,
    slot: u8,
    func: u8,
    handler: extern "C" fn(arg: usize), 
    arg: usize,
    entry_idx: usize,
    target_core: usize) -> Result<u8, ()> {

    let st = ALLOC_STATE.lock();
    let vecs = st.handles.get(&handle.id).ok_or(())?;
    let meta = vecs.get(idx).ok_or(())?;
    let v = meta.vector;

    let arch = ARCH_FUNCS.get().ok_or(())?;
    (arch.register_irq_entry)(v, handler, arg);

    // Setup MSI-X in PCI config space and table
    crate::drivers::pci::pci_setup_msix_entry(bus, slot, func, v, target_core, entry_idx)?;

    Ok(v)
}


pub fn msi_free(handle: MsiHandle) {
    let mut st = ALLOC_STATE.lock();
    if let Some(vecs) = st.handles.remove(&handle.id) {
        if let Some(arch) = ARCH_FUNCS.get() {
            for meta in vecs {
                (arch.free_vector)(meta.vector);
            }
        }
    }	    
}
