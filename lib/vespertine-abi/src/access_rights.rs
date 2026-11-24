pub use crate::define_bitflags;

define_bitflags! {
    pub struct AccessRights(u8) {
        READ            = 1 << 0;
        WRITE           = 1 << 1;
        EXECUTE         = 1 << 2;
        CREATE          = 1 << 3;
        MUTATE          = 1 << 4;
        TRAVERSE        = 1 << 5;
        LIST            = 1 << 6;
        REMOVE          = 1 << 7;
    }
}

impl core::fmt::Display for AccessRights {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let flags = [
            (Self::READ, "READ"),
            (Self::WRITE, "WRITE"),
            (Self::EXECUTE, "EXECUTE"),
            (Self::CREATE, "CREATE"),
            (Self::MUTATE, "MUTATE"),
            (Self::TRAVERSE, "TRAVERSE"),
            (Self::LIST, "LIST"),
            (Self::REMOVE, "REMOVE"),
        ];

        if self.is_empty() {
            return f.write_str("NONE");
        }

        let mut separator = "";
        for (flag, name) in flags {
            if self.contains(flag) {
                f.write_str(separator)?;
                f.write_str(name)?;
                separator = "|";
            }
        }
        Ok(())
    }
}
