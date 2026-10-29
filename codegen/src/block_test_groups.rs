// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2026.

use crate::block::{
    bus_doc_comment, field_struct_doc_comment, interface_doc_comment, lengths_doc_comment,
    only_doc_comment, real_doc_comment,
};
use crate::{new_doc_comment, register_map, test_util::assert_tokens_eq, Env::ProcMacro};
use quote::quote;
use syn::parse_quote;

/// Tests the code generated for a block with register groups. The parts of the output that do not
/// depend on groups (Bus, lengths, offset tests, the register structs) are the same as for
/// ungrouped blocks; see block_test_all_fields for an explanation of those.
#[test]
fn groups() {
    let input = quote! {
        ::tock_registers
        #[buses(Mmio32)]
        pub foo {
            /// Dma docs
            #[group(Dma)],
            0 => a: u8 { Read },
            #[group = Dma] 1 => b: u8 { Write },
            #[group = Dma] 2 => c: other,
        }
    };
    let interface_comment = interface_doc_comment();
    let interface_only_comment = only_doc_comment(&parse_quote![Interface]);
    let dma_interface_only_comment = only_doc_comment(&parse_quote![DmaInterface]);
    let lengths_comment = lengths_doc_comment();
    let bus_comment = bus_doc_comment();
    let real_comment = real_doc_comment();
    let new_comment = new_doc_comment();
    let a_comment = field_struct_doc_comment(&parse_quote![a]);
    let b_comment = field_struct_doc_comment(&parse_quote![b]);
    let expected = quote! {
        pub mod foo {
            #![allow(non_camel_case_types)] use super::*;
            // Each group has its own trait, containing only that group's fields. The default
            // group's trait is `Interface`.
            #interface_comment pub trait Interface {
                type a<'s>: ::tock_registers::Register<DataType = u8> + Read where Self: 's;
                fn a(&self) -> Self::a<'_>;
            }
            impl<T: ::tock_registers::internal::core::ops::Deref<Target: Interface>>
                Interface for T
            {
                type a<'s> = <T::Target as Interface>::a<'s> where Self: 's;
                fn a(&self) -> Self::a<'_> { self.deref().a() }
            }
            // The narrowing wrapper. It forwards only Interface, and deliberately does not
            // implement Deref, so the Deref blanket impls of the other groups' traits do not apply
            // to it.
            #interface_only_comment #[derive(Clone)] pub struct InterfaceOnly<T>(T);
            impl<T: Interface> InterfaceOnly<T> {
                #[doc = "Wraps `inner`, exposing only its [Interface] implementation."]
                pub const fn new(inner: T) -> Self { Self(inner) }
            }
            impl<T: ::tock_registers::internal::core::marker::Copy>
                ::tock_registers::internal::core::marker::Copy for InterfaceOnly<T> {}
            impl<T: Interface> Interface for InterfaceOnly<T> {
                type a<'s> = <T as Interface>::a<'s> where Self: 's;
                fn a(&self) -> Self::a<'_> { self.0.a() }
            }
            // The group's doc comments are the trait's doc comments.
            #[doc = r" Dma docs"] pub trait DmaInterface {
                type b<'s>: ::tock_registers::Register<DataType = u8> + Write where Self: 's;
                fn b(&self) -> Self::b<'_>;
                type c<'s>: other::Interface where Self: 's;
                fn c(&self) -> Self::c<'_>;
            }
            impl<T: ::tock_registers::internal::core::ops::Deref<Target: DmaInterface>>
                DmaInterface for T
            {
                type b<'s> = <T::Target as DmaInterface>::b<'s> where Self: 's;
                fn b(&self) -> Self::b<'_> { self.deref().b() }
                type c<'s> = <T::Target as DmaInterface>::c<'s> where Self: 's;
                fn c(&self) -> Self::c<'_> { self.deref().c() }
            }
            #dma_interface_only_comment #[derive(Clone)] pub struct DmaInterfaceOnly<T>(T);
            impl<T: DmaInterface> DmaInterfaceOnly<T> {
                #[doc = "Wraps `inner`, exposing only its [DmaInterface] implementation."]
                pub const fn new(inner: T) -> Self { Self(inner) }
            }
            impl<T: ::tock_registers::internal::core::marker::Copy>
                ::tock_registers::internal::core::marker::Copy for DmaInterfaceOnly<T> {}
            impl<T: DmaInterface> DmaInterface for DmaInterfaceOnly<T> {
                type b<'s> = <T as DmaInterface>::b<'s> where Self: 's;
                fn b(&self) -> Self::b<'_> { self.0.b() }
                type c<'s> = <T as DmaInterface>::c<'s> where Self: 's;
                fn c(&self) -> Self::c<'_> { self.0.c() }
            }
            // Read by blocks that refer to this one, to enforce that they only do so from a
            // non-default group.
            #[doc = r" Whether this layout declares register groups."]
            pub const HAS_GROUPS: bool = true;
            #lengths_comment pub mod lengths {}
            #bus_comment #[allow(clippy::trait_duplication_in_bounds)]
            pub trait Bus: ::tock_registers::Address + ::tock_registers::DataTypeBus<u8>
                + ::tock_registers::DataTypeBus<u8> + other::Bus + sealed::Bus
            {
                const SIZE: usize;
                const a_offset: usize = 0;
                const b_offset: usize = 1;
                const c_offset: usize = 2;
            }
            impl Bus for Mmio32 {
                const SIZE: usize = 2 + <other::Real<Mmio32> as ::tock_registers::Span>::SIZE;
            }
            impl sealed::Bus for Mmio32 {}
            impl<B: Bus> Bus for ::tock_registers::BorrowedBus<'_, B> {
                const SIZE: usize = <B as Bus>::SIZE;
            }
            impl<B: Bus> sealed::Bus for ::tock_registers::BorrowedBus<'_, B> {}
            // `c` refers to another layout, but it is in a non-default group, so there is no
            // `assert!(!other::HAS_GROUPS)` check.
            const _: () = {
                assert!(0 == ::tock_registers::internal::core::convert::identity(0),
                    "offset mismatch for bus Mmio32");
                assert!(1 == ::tock_registers::internal::core::convert::identity(0 + <
                    real_a<Mmio32> as ::tock_registers::Span>::SIZE),
                    "offset mismatch for bus Mmio32");
                assert!(2 == ::tock_registers::internal::core::convert::identity(1 + <
                    real_b<Mmio32> as ::tock_registers::Span>::SIZE),
                    "offset mismatch for bus Mmio32");
            };
            mod sealed { pub trait Bus {} }
            #real_comment #[derive(Clone)] pub struct Real<B: Bus> {
                address: B,
                _phantom: ::tock_registers::internal::RealPhantom,
            }
            impl<B: Bus> Real<B> {
                #new_comment pub const unsafe fn new(address: B) -> Self {
                    Self { address, _phantom: ::tock_registers::internal::RealPhantom::new() }
                }
            }
            impl<B: Bus> ::tock_registers::internal::core::marker::Copy for Real<B> {}
            // Real implements every group's trait.
            impl<B: Bus> Interface for Real<B>
                where real_a<B>: ::tock_registers::Register<DataType = u8> + Read,
            {
                type a<'s> = real_a<B> where Self: 's;
                fn a(&self) -> Self::a<'_> {
                    unsafe { Self::a::new(self.address.byte_add(<B as Bus>::a_offset)) }
                }
            }
            impl<B: Bus> DmaInterface for Real<B>
                where real_b<B>: ::tock_registers::Register<DataType = u8> + Write,
                      other::Real<B>: other::Interface,
            {
                type b<'s> = real_b<B> where Self: 's;
                fn b(&self) -> Self::b<'_> {
                    unsafe { Self::b::new(self.address.byte_add(<B as Bus>::b_offset)) }
                }
                type c<'s> = other::Real<B> where Self: 's;
                fn c(&self) -> Self::c<'_> {
                    unsafe { Self::c::new(self.address.byte_add(<B as Bus>::c_offset)) }
                }
            }
            unsafe impl<B: Bus> ::tock_registers::Span for Real<B> {
                type Address = B;
                const SIZE: usize = <B as Bus>::SIZE;
                unsafe fn with_addr(address: B) -> Self {
                    Self { address, _phantom: ::tock_registers::internal::RealPhantom::new() }
                }
                type Borrowed<'b> = Real<::tock_registers::BorrowedBus<'b, B>>;
            }
            #a_comment #[derive(Clone)] pub struct real_a<B: Bus> {
                address: B,
                _phantom: ::tock_registers::internal::RealPhantom,
            }
            impl<B: Bus> real_a<B> {
                #new_comment pub const unsafe fn new(address: B) -> Self {
                    Self { address, _phantom: ::tock_registers::internal::RealPhantom::new() }
                }
            }
            impl<B: Bus> ::tock_registers::internal::core::marker::Copy for real_a<B> {}
            unsafe impl<B: Bus> ::tock_registers::Span for real_a<B> {
                type Address = B;
                const SIZE: usize = <B as ::tock_registers::DataTypeBus<u8>>::PADDED_SIZE;
                unsafe fn with_addr(address: B) -> Self {
                    Self { address, _phantom: ::tock_registers::internal::RealPhantom::new() }
                }
                type Borrowed<'b> = real_a<::tock_registers::BorrowedBus<'b, B>>;
            }
            impl<B: Bus> ::tock_registers::Register for real_a<B> {
                type DataType = u8;
            }
            Read!(real_impl, real_a, u8,,);
            #b_comment #[derive(Clone)] pub struct real_b<B: Bus> {
                address: B,
                _phantom: ::tock_registers::internal::RealPhantom,
            }
            impl<B: Bus> real_b<B> {
                #new_comment pub const unsafe fn new(address: B) -> Self {
                    Self { address, _phantom: ::tock_registers::internal::RealPhantom::new() }
                }
            }
            impl<B: Bus> ::tock_registers::internal::core::marker::Copy for real_b<B> {}
            unsafe impl<B: Bus> ::tock_registers::Span for real_b<B> {
                type Address = B;
                const SIZE: usize = <B as ::tock_registers::DataTypeBus<u8>>::PADDED_SIZE;
                unsafe fn with_addr(address: B) -> Self {
                    Self { address, _phantom: ::tock_registers::internal::RealPhantom::new() }
                }
                type Borrowed<'b> = real_b<::tock_registers::BorrowedBus<'b, B>>;
            }
            impl<B: Bus> ::tock_registers::Register for real_b<B> {
                type DataType = u8;
            }
            Write!(real_impl, real_b, u8,,);
        }
    };
    assert_tokens_eq(register_map(input, ProcMacro).unwrap(), expected);
}
