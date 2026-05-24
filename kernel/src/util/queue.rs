#[macro_export]
macro_rules! impl_queue_methods {
    ($queue_type:ty, $item_type:ty, $head_field:ident, $tail_field:ident) => {
        impl $queue_type {
            pub fn push(&mut self, item: *mut $item_type) {
                if item.is_null() {
                    return;
                }
                unsafe {
                    (*item).next = null_mut();
                    if self.$tail_field.is_null() {
                        self.$head_field = item;
                        self.$tail_field = item;
                    } else {
                        (*self.$tail_field).next = item;
                        self.$tail_field = item;
                    }
                    self.queue_length.fetch_add(1, Ordering::Relaxed);
                }
            }

            pub fn pop(&mut self) -> *mut $item_type {
                unsafe {
                    if self.$head_field.is_null() {
                        return null_mut();
                    }

                    let ret = self.$head_field;
                    self.$head_field = (*ret).next;
                    if self.$head_field.is_null() {
                        self.$tail_field = null_mut();
                    }
                    (*ret).next = null_mut();
                    self.queue_length.fetch_sub(1, Ordering::Relaxed);
                    ret
                }
            }
        }
    };
}
