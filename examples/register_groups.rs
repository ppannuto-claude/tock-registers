// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2026.

//! Example of using register groups to restrict access to a peripheral's DMA registers.
//!
//! `AesDmaManager` owns the whole peripheral and lends out an `aes::InterfaceOnly` view, which
//! implements `aes::Interface` but not `aes::DmaInterface`, so holders of the view cannot start or
//! complete a DMA operation.

use core::cell::Cell;
use tock_registers::{register_map_mmio32, Read, Write};

register_map_mmio32! {
    /// A simplified AES peripheral that encrypts a 16-byte buffer in place.
    pub aes {
        /// Registers that control when the hardware accesses memory. Whoever holds
        /// `aes::DmaInterface` must only start an operation when `ptr` points to a 16-byte buffer
        /// that the hardware may read and write until `event_done` is set. `event_done` is in this
        /// group because the buffer is released when it is set.
        #[group(Dma)],
        /// Writing 1 starts encrypting the buffer at `ptr`.
        #[group = Dma] 0 => task_start: u32 { Write },
        /// Set to 1 by the hardware when encryption finishes. Write 0 to clear.
        #[group = Dma] 4 => event_done: u32 { Read, Write },
        /// Address of the buffer to encrypt.
        #[group = Dma] 8 => ptr: u32 { Write },
        /// Key slot to encrypt with. Does not affect what memory the hardware accesses.
        12 => key_slot: u32 { Read, Write },
    }
}

/// Owns the AES peripheral and the buffer the hardware is using.
pub struct AesDmaManager<R: aes::Interface + aes::DmaInterface> {
    all_regs: R,
    buffer: Cell<Option<&'static mut [u8; 16]>>,
}

impl<R: aes::Interface + aes::DmaInterface> AesDmaManager<R> {
    /// # Safety
    /// `all_regs` must be the only accessor for this peripheral's Dma-group registers.
    pub unsafe fn new(all_regs: R) -> Self {
        Self {
            all_regs,
            buffer: Cell::new(None),
        }
    }

    /// Starts encrypting `buffer` in place. Returns the buffer if an operation is already
    /// ongoing.
    pub fn start(&self, buffer: &'static mut [u8; 16]) -> Result<(), &'static mut [u8; 16]> {
        if let Some(old) = self.buffer.take() {
            self.buffer.set(Some(old));
            return Err(buffer);
        }
        self.all_regs.event_done().set(0);
        // Assumes a 32-bit address space. A real manager would also need a fence before the start
        // trigger.
        self.all_regs.ptr().set(buffer.as_mut_ptr() as usize as u32);
        self.buffer.set(Some(buffer));
        self.all_regs.task_start().set(1);
        Ok(())
    }

    /// Returns the buffer once encryption has finished.
    pub fn finish(&self) -> Option<&'static mut [u8; 16]> {
        if self.all_regs.event_done().get() == 0 {
            return None;
        }
        self.buffer.take()
    }

    /// Returns a handle to the peripheral's non-DMA registers.
    pub fn registers(&self) -> aes::InterfaceOnly<&R> {
        aes::InterfaceOnly::new(&self.all_regs)
    }
}

/// Code that only needs the non-DMA registers can be handed the narrowed view.
pub fn select_key<R: aes::Interface>(registers: R, slot: u32) {
    registers.key_slot().set(slot);
}

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use tock_registers::{FakeRegister, NoAccess, Safe};

    /// Fake AES peripheral that finishes each operation instantly (without touching the buffer).
    #[derive(Default)]
    struct FakeAes {
        event_done: Cell<u32>,
        ptr_written: Cell<bool>,
        key_slot: Cell<u32>,
    }

    impl aes::Interface for FakeAes {
        type key_slot<'s> = FakeRegister<&'s Self, u32, Safe, Safe>;
        fn key_slot(&self) -> Self::key_slot<'_> {
            FakeRegister::new(self)
                .on_read(|this| this.key_slot.get())
                .on_write(|this, value| this.key_slot.set(value))
        }
    }

    impl aes::DmaInterface for FakeAes {
        type event_done<'s> = FakeRegister<&'s Self, u32, Safe, Safe>;
        fn event_done(&self) -> Self::event_done<'_> {
            FakeRegister::new(self)
                .on_read(|this| this.event_done.get())
                .on_write(|this, value| this.event_done.set(value))
        }
        type task_start<'s> = FakeRegister<&'s Self, u32, NoAccess, Safe>;
        fn task_start(&self) -> Self::task_start<'_> {
            FakeRegister::new(self).on_write(|this, _| {
                assert!(this.ptr_written.get(), "started without a buffer");
                this.event_done.set(1);
            })
        }
        type ptr<'s> = FakeRegister<&'s Self, u32, NoAccess, Safe>;
        fn ptr(&self) -> Self::ptr<'_> {
            FakeRegister::new(self).on_write(|this, _| this.ptr_written.set(true))
        }
    }

    #[test]
    fn encrypt() {
        let fake = Rc::new(FakeAes::default());
        // Safety: this is the only accessor for `fake`'s Dma-group registers.
        let manager = unsafe { AesDmaManager::new(fake.clone()) };
        select_key(manager.registers(), 0x5a);
        assert_eq!(fake.key_slot.get(), 0x5a);

        assert!(manager.finish().is_none());
        manager.start(Box::leak(Box::new([0; 16]))).unwrap();
        let buffer = manager.finish().unwrap();
        // Safety: `buffer` came from Box::leak, and the manager no longer holds it.
        drop(unsafe { Box::from_raw(buffer) });
    }
}
