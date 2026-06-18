use crate::define_bitflags;

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
