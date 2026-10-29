// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2026.
// Copyright Better Bytes 2026.

use crate::ast::{Field, FieldDef, Group, Layout, PerBusInt};
use crate::{new_doc_comment, register_definition, Env};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{spanned::Spanned, Ident, Index, Path, TypePath};

/// Generates the module for a register block.
pub fn generate(
    env: Env,
    tock_registers: &Path,
    layout: &Layout,
    groups: &[Group],
    fields: &[Field],
) -> TokenStream {
    // At a high level, this function has:
    //
    // 1. A set of variable declarations (of type TokenStream or Vec<TokenStream>)
    // 2. A loop that loops through each field, adding to the variable declarations.
    // 3. A final quote! invocation that combines the variable declarations into the output code.
    //
    // This pattern avoids the need to have separate loops for each portion of the generated code,
    // which removes a lot of code duplication.
    //
    // This is generally more complex than single::generate, so if you need to understand/modify
    // both functions, consider working on single::generate first. That knowledge will transfer
    // over to this function.
    //
    // The advice from single::generate's comment about looking at the generated code first, then
    // tracing backwards through the final quote! invocation to find the relevant variables works
    // in this function as well. block_test_all_fields is a good test to look at first.

    // Step 1: variable declarations
    let env_allows = match env {
        Env::External => quote![,dead_code,non_upper_case_globals],
        Env::ProcMacro => quote![],
    };
    let docs = &layout.docs;
    let visibility = &layout.visibility;
    let name = &layout.name;
    // Generated code for each register group, starting with the default group.
    let mut group_tokens: Vec<_> = [None]
        .into_iter()
        .chain(groups.iter().map(Some))
        .map(GroupTokens::new)
        .collect();
    let mut nesting_checks = TokenStream::new();
    let lengths_comment = lengths_doc_comment();
    let mut len_definitions = TokenStream::new();
    let bus_comment = bus_doc_comment();
    let mut bus_bounds = TokenStream::new();
    let mut bus_offset_decls = TokenStream::new();
    let buses = layout.bus.as_slice();
    // cumulative_sizes is empty if the current cumulative size is unknown (due to a padding field
    // with no specified size).
    let mut cumulative_sizes: Vec<_> = (0..buses.len()).map(|_| quote![0]).collect();
    let mut bus_offset_defs: Vec<_> = (0..buses.len()).map(|_| TokenStream::new()).collect();
    let mut borrowed_bus_defs = TokenStream::new();
    let mut offset_tests = TokenStream::new();
    let bus_default = layout.bus.generic_default();
    let real_comment = real_doc_comment();
    let new_comment = new_doc_comment();
    let mut real_structs = TokenStream::new();

    // Step 2: Loop through each field, update the variables.
    for field in fields.iter() {
        // This `match` statement handles padding and `continue`s to the next iteration on padding
        // fields, so the rest of the body of this loop does not need to special-case for padding.
        let (docs, aliased, group, name, register) = match &field.field_def {
            FieldDef::Padding(sizes) => {
                add_offset_tests(
                    tock_registers,
                    &mut offset_tests,
                    buses,
                    &cumulative_sizes,
                    &field.offsets,
                );
                cumulative_sizes.clear();
                if let Some(sizes) = sizes {
                    for bus_idx in 0..buses.len() {
                        let offset = &field.offsets[bus_idx];
                        let size = &sizes[bus_idx];
                        cumulative_sizes.push(quote![#offset + #size]);
                    }
                }
                continue;
            }
            FieldDef::Register {
                docs,
                aliased,
                group,
                name,
                spec,
            } => (docs, *aliased, group, name, spec),
        };
        let GroupTokens {
            group: field_group,
            trait_name,
            interface_fields,
            deref_impl_items,
            interface_bounds,
            interface_impl_items,
            only_impl_items,
        } = group_tokens
            .iter_mut()
            .find(|g| g.group.map(|g| &g.name) == group.as_ref())
            .expect("undeclared group");
        // The rest of this loop body is for register fields. It consists of a series of
        // conditionals and loops that all switch/iterate on a different aspect of the field.
        let element_type = &register.element_type;
        let mut interface_bound;
        let mut real;
        let mut real_per_bus: Vec<_>;
        // If statement that handles differences between register definitions (which have
        // operations) and register references (which do not).
        if let Some(operations) = &register.operations {
            interface_bound =
                quote![#tock_registers::Register<DataType = #element_type> #(+ #operations)*];
            let real_name = format_ident!("real_{name}");
            real = quote![#real_name<B>];
            real_per_bus = buses.iter().map(|b| quote![#real_name<#b>]).collect();
            let bus_trait = quote![#tock_registers::DataTypeBus<#element_type>];
            bus_bounds.extend(quote![+ #bus_trait]);
            real_structs.extend(register_definition(
                tock_registers,
                field_struct_doc_comment(name),
                &bus_default,
                &real_name,
                register,
                operations,
            ));
        } else {
            interface_bound = quote![#element_type::Interface];
            real = quote![#element_type::Real<B>];
            real_per_bus = buses
                .iter()
                .map(|b| quote![#element_type::Real<#b>])
                .collect();
            bus_bounds.extend(quote_spanned![element_type.span()=>+ #element_type::Bus]);
            // This field's type is the referenced layout's Real, which exposes all of that layout's
            // groups, so the default group (and InterfaceOnly) must not contain it.
            if field_group.is_none() {
                let msg = format!(
                    "`{name}` refers to a layout with register groups, so it must be assigned to a \
                     non-default group"
                );
                nesting_checks.extend(quote_spanned![element_type.span()=>
                    assert!(!#element_type::HAS_GROUPS, #msg);]);
            }
        };
        interface_bounds.extend(quote![#real: #interface_bound,]);
        // match that handles the difference between scalar registers, non-nested array registers,
        // and nested array registers.
        let len_types_sizes = match register.array_sizes.as_slice() {
            [] => vec![],
            [len] => {
                len_definitions.extend(quote![pub enum #name {}]);
                vec![(quote![#name], len)]
            }
            nested => {
                len_definitions.extend(quote![pub enum #name<const N: usize> {}]);
                nested
                    .iter()
                    .enumerate()
                    .map(|(n, s)| (quote![#name<#n>], s))
                    .collect()
            }
        };
        // Loop that runs once for each level of array nesting.
        for (len_type, size) in len_types_sizes {
            interface_bound = quote![#tock_registers::RegisterArray<lengths::#len_type, Element: #interface_bound>];
            len_definitions.extend(quote! {
                impl #tock_registers::array::Len for #len_type { const LEN: usize = #size; }
            });
            real = quote![#tock_registers::RealRegisterArray<#real, lengths::#len_type>];
            for r in &mut real_per_bus {
                *r = quote![#tock_registers::RealRegisterArray<#r, lengths::#len_type>];
            }
        }
        interface_fields.extend(quote! {
            type #name<'s>: #interface_bound where Self: 's;
            #(#docs)* fn #name(&self) -> Self::#name<'_>;
        });
        deref_impl_items.extend(quote! {
            type #name<'s> = <T::Target as #trait_name>::#name<'s> where Self: 's;
            fn #name(&self) -> Self::#name<'_> { self.deref().#name() }
        });
        // `self.0.#name` would lex `0.` as a float literal.
        let inner = Index::from(0);
        only_impl_items.extend(quote! {
            type #name<'s> = <T as #trait_name>::#name<'s> where Self: 's;
            fn #name(&self) -> Self::#name<'_> { self.#inner.#name() }
        });
        let name_offset = format_ident!("{name}_offset");
        // match that handles the difference between scalar offset definitions and array offset
        // definitions (moves the value of the name_offset fields between the Bus trait definition
        // and impls).
        bus_offset_decls.extend(match &field.offsets {
            PerBusInt::Array(offsets) => {
                for (bus_idx, offset) in offsets.iter().enumerate() {
                    bus_offset_defs[bus_idx].extend(quote![const #name_offset: usize = #offset;]);
                }
                borrowed_bus_defs
                    .extend(quote![const #name_offset: usize = <B as Bus>::#name_offset;]);
                quote![const #name_offset: usize;]
            }
            PerBusInt::Single(offset) => quote![const #name_offset: usize = #offset;],
        });
        // if that handles aliased vs. non-aliased fields.
        if !aliased {
            add_offset_tests(
                tock_registers,
                &mut offset_tests,
                buses,
                &cumulative_sizes,
                &field.offsets,
            );
            cumulative_sizes.clear();
            for (bus_idx, bus_real) in real_per_bus.into_iter().enumerate() {
                let offset = &field.offsets[bus_idx];
                cumulative_sizes.push(quote![#offset + <#bus_real as #tock_registers::Span>::SIZE]);
            }
        }
        interface_impl_items.extend(quote! {
            type #name<'s> = #real where Self: 's;
            fn #name(&self) -> Self::#name<'_> {
                // Safety (see crate::new_doc_comment() for requirements):
                // 1. When Self::new was called to construct `self`, the caller guaranteed that the
                //    passed address points to registers on the bus of type B.
                // 2. The definition of this register block is correct (guaranteed by the caller of
                //    Self::new), which guarantees that a register corresponding to the #real type
                //    exists at B::name_offset. This also guarantees that the byte_add does not
                //    leave the register block.
                // 3. The user of this struct is responsible for using the entire register block in
                //    a way that avoids data races, which includes the responsibility to avoid data
                //    races on individual fields of the register block.
                unsafe {
                    Self::#name::new(self.address.byte_add(<B as Bus>::#name_offset))
                }
            }
        });
    }

    // Step 3: the final quote! call that puts everything together.
    let has_groups = !groups.is_empty();
    let mut group_traits = TokenStream::new();
    let mut real_group_impls = TokenStream::new();
    for tokens in group_tokens {
        let GroupTokens {
            group,
            trait_name,
            interface_fields,
            deref_impl_items,
            interface_bounds,
            interface_impl_items,
            only_impl_items,
        } = tokens;
        let trait_docs = match group {
            None => interface_doc_comment(),
            Some(Group { docs, .. }) if !docs.is_empty() => quote![#(#docs)*],
            Some(Group { name, .. }) => {
                let msg = format!("Trait representing the `{name}` group of this register block.");
                quote![#[doc = #msg]]
            }
        };
        group_traits.extend(quote! {
            #trait_docs pub trait #trait_name { #interface_fields }
            impl<T: #tock_registers::internal::core::ops::Deref<Target: #trait_name>>
                #trait_name for T { #deref_impl_items }
        });
        real_group_impls.extend(quote! {
            impl<B: Bus> #trait_name for Real<B> where #interface_bounds {
                #interface_impl_items
            }
        });
        // The narrowing wrapper is only useful when there is more than one group.
        if !has_groups {
            continue;
        }
        let only_name = format_ident!("{trait_name}Only");
        let only_docs = only_doc_comment(&trait_name);
        let new_msg = format!("Wraps `inner`, exposing only its [{trait_name}] implementation.");
        group_traits.extend(quote! {
            #only_docs #[derive(Clone)] pub struct #only_name<T>(T);
            impl<T: #trait_name> #only_name<T> {
                #[doc = #new_msg] pub const fn new(inner: T) -> Self { Self(inner) }
            }
            impl<T: #tock_registers::internal::core::marker::Copy>
                #tock_registers::internal::core::marker::Copy for #only_name<T> {}
            impl<T: #trait_name> #trait_name for #only_name<T> { #only_impl_items }
        });
    }
    quote! {
        #(#docs)*
        #visibility mod #name {
            #![allow(non_camel_case_types #env_allows)] use super::*;
            #group_traits
            /// Whether this layout declares register groups.
            pub const HAS_GROUPS: bool = #has_groups;
            #lengths_comment pub mod lengths { #len_definitions }
            #bus_comment #[allow(clippy::trait_duplication_in_bounds)]
            pub trait Bus: #tock_registers::Address #bus_bounds + sealed::Bus {
                const SIZE: usize;
                #bus_offset_decls
            }
            #(
                impl Bus for #buses {
                    const SIZE: usize = #cumulative_sizes;
                    #bus_offset_defs
                }
                impl sealed::Bus for #buses {}
            )*
            impl<B: Bus> Bus for #tock_registers::BorrowedBus<'_, B> {
                const SIZE: usize = <B as Bus>::SIZE;
                #borrowed_bus_defs
            }
            impl<B: Bus> sealed::Bus for #tock_registers::BorrowedBus<'_, B> {}
            const _: () = { #offset_tests #nesting_checks };
            mod sealed { pub trait Bus {} }
            #real_comment #[derive(Clone)] pub struct Real<B: Bus #bus_default> {
                address: B,
                _phantom: #tock_registers::internal::RealPhantom,
            }
            impl<B: Bus> Real<B> {
                #new_comment pub const unsafe fn new(address: B) -> Self {
                    Self { address, _phantom: #tock_registers::internal::RealPhantom::new() }
                }
            }
            impl<B: Bus> #tock_registers::internal::core::marker::Copy for Real<B> {}
            #real_group_impls
            // Safety: Our size calculation for Bus::SIZE is correct. Bus is sealed, so there can
            // be no other implementations of Bus.
            unsafe impl<B: Bus> #tock_registers::Span for Real<B> {
                type Address = B;
                const SIZE: usize = <B as Bus>::SIZE;
                unsafe fn with_addr(address: B) -> Self {
                    Self { address, _phantom: #tock_registers::internal::RealPhantom::new() }
                }
                type Borrowed<'b> = Real<#tock_registers::BorrowedBus<'b, B>>;
            }
            #real_structs
        }
    }
}

/// Generated code for one register group.
struct GroupTokens<'g> {
    /// None for the default group.
    group: Option<&'g Group>,
    trait_name: Ident,
    interface_fields: TokenStream,
    deref_impl_items: TokenStream,
    interface_bounds: TokenStream,
    interface_impl_items: TokenStream,
    only_impl_items: TokenStream,
}

impl<'g> GroupTokens<'g> {
    fn new(group: Option<&'g Group>) -> Self {
        Self {
            group,
            trait_name: match group {
                None => format_ident!("Interface"),
                Some(group) => format_ident!("{}Interface", group.name),
            },
            interface_fields: TokenStream::new(),
            deref_impl_items: TokenStream::new(),
            interface_bounds: TokenStream::new(),
            interface_impl_items: TokenStream::new(),
            only_impl_items: TokenStream::new(),
        }
    }
}

pub fn only_doc_comment(trait_name: &Ident) -> TokenStream {
    let msg = format!(
        "Narrowed view of a register block that implements only [{trait_name}]. `T` may be owned \
         (`R`) or borrowed (`&R`)."
    );
    quote![#[doc = #msg]]
}

pub fn interface_doc_comment() -> TokenStream {
    quote! {
        /// Trait representing this register block. Driver code can use this trait to work with
        /// both real hardware and fake implementations of the peripheral (for unit testing).
    }
}

pub fn lengths_doc_comment() -> TokenStream {
    quote! {
        /// Phantom types encoding the lengths of register arrays in this block.
        ///
        /// Driver code can use these as type parameters when writing generic functions
        /// over register arrays in this block, for example:
        ///
        /// ```ignore
        /// fn process<R: RegisterArray<my_block::lengths::my_array>>(regs: R) { ... }
        /// ```
    }
}

pub fn bus_doc_comment() -> TokenStream {
    quote! {
        /// Buses supported by this register block.
    }
}

pub fn real_doc_comment() -> TokenStream {
    quote! {
        /// Struct implementing [Interface] for use with the real hardware.
    }
}

pub fn field_struct_doc_comment(name: &Ident) -> TokenStream {
    let msg = format!("Struct that provides access to the `{name}` register on real hardware.");
    quote![#[doc = #msg]]
}

/// Adds offset tests for a field with the given offsets. If the current cumulative size is unknown
/// (because of a padding field with no specified size), this does nothing.
fn add_offset_tests(
    tock_registers: &Path,
    offset_tests: &mut TokenStream,
    buses: &[TypePath],
    cumulative_sizes: &[TokenStream],
    offsets: &PerBusInt,
) {
    for (bus_idx, cumulative_size) in cumulative_sizes.iter().enumerate() {
        let offset = &offsets[bus_idx];
        let bus = &buses[bus_idx].path.segments.last().expect("empty bus path");
        let msg = format!("offset mismatch for bus {}", bus.ident);
        offset_tests.extend(quote_spanned![offset.span()=>assert!(#offset ==
            #tock_registers::internal::core::convert::identity(#cumulative_size), #msg);]);
    }
}
