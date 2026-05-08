use core::slice::from_raw_parts;
use crate::boot::RSDP_REQUEST;
use crate::HHDMOFFSET;

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct RSDP_Descriptor {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct RSDP_v2_Descriptor {
    v1: RSDP_Descriptor,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    reserved: [u8; 3]
}

#[derive(Copy, Clone)]
pub enum Rsdp {
    V1(RSDP_Descriptor),
    V2(RSDP_v2_Descriptor),
}

pub enum AcpiRoot {
    RSDT(usize),
    XSDT(usize),
}

impl Rsdp {
    pub fn get() -> Rsdp {
        let resp_addr = RSDP_REQUEST.response()
            .expect("COULD NOT GET RSDP ADDRESS FROM LIMINE")
            .address;
        
        let rsdp_ptr = resp_addr as *const RSDP_Descriptor;

        unsafe {
            if (*rsdp_ptr).revision >= 2 {
                let rsdp_v2_ptr = resp_addr as *const RSDP_v2_Descriptor;
                Rsdp::V2(*rsdp_v2_ptr)
            } else {
                Rsdp::V1(*rsdp_ptr)
            }
        }
    }

    fn validate(self) -> Option<()> {
        let checksum = |ptr: *const u8, len: usize| -> u8 {
            unsafe {
                from_raw_parts(ptr, len)
                    .iter()
                    .fold(0u8, |acc, &x| acc.wrapping_add(x))
            }
        };

        match self {
            Rsdp::V1(desc) => {
                if checksum(&desc as *const _ as *const u8, 20) == 0 {
                    Some(())
                } else {
                    None
                }
            },
            Rsdp::V2(desc) => {
                let ptr = &desc as *const _ as *const u8;
                
                let v1_sum = checksum(ptr, 20);
                let v2_sum = checksum(ptr, 36);
                
                if v1_sum == 0 && v2_sum == 0 { 
                    Some(()) 
                } else { 
                    None 
                }
            }
        }
    }

    pub fn get_table(self) -> Option<AcpiRoot> {
        self.validate()?;
        match self {
            Rsdp::V1(desc) => Some(AcpiRoot::RSDT(desc.rsdt_address as usize + *HHDMOFFSET)),
            Rsdp::V2(desc) => {
                if desc.xsdt_address == 0 {
                    Some(AcpiRoot::RSDT(desc.v1.rsdt_address as usize + *HHDMOFFSET))
                } else {
                    Some(AcpiRoot::XSDT(desc.xsdt_address as usize + *HHDMOFFSET))
                }
            }
        }
    }
}
