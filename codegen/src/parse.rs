// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2026.
// Copyright Better Bytes 2026.

//! Input parser. The best reference for what this does is the [ast] module, as the doc comment on
//! each AST type shows that type's definition syntax.

use crate::ast::{BusAttr, Field, FieldDef, Group, Input, Layout, PerBusInt, RegisterSpec, Value};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token::{Brace, Bracket};
use syn::{
    braced, bracketed, Attribute, Error, Expr, Ident, LitInt, Meta, MetaNameValue, Result, Token,
    Type,
};

impl Parse for Input {
    fn parse(input: ParseStream) -> Result<Input> {
        let tock_registers = input.parse()?;
        // Parse attributes that apply to all layouts.
        let (docs, bus) = layout_attributes(Attribute::parse_inner(input)?)?;
        if let Some(doc) = docs.last() {
            return Err(Error::new(
                doc.span(),
                "inner doc comments are not supported",
            ));
        }
        let punctuated = Punctuated::<Layout, Token![,]>::parse_terminated(input)?;
        let mut layouts = Vec::with_capacity(punctuated.len());
        for mut layout in punctuated {
            // Prepend the global (inner attribute) docs to each Layout's local (outer attribute)
            // docs).
            layout.docs = docs.iter().cloned().chain(layout.docs).collect();
            // Combine the Layout's buses specification with the global buses specification.
            if layout.bus.as_slice().is_empty() {
                if bus.as_slice().is_empty() {
                    return Err(Error::new(layout.name.span(), "no bus specified"));
                }
                layout.bus = bus.clone();
            }
            let Value::Block { fields, .. } = &layout.value else {
                layouts.push(layout);
                continue;
            };
            for field in fields {
                if let PerBusInt::Array(offsets) = &field.offsets {
                    if offsets.len() != layout.bus.len() {
                        return Err(Error::new(
                            offsets[0].span(),
                            format!(
                                "number of offsets ({}) does not match number of buses ({})",
                                offsets.len(),
                                layout.bus.len()
                            ),
                        ));
                    };
                }
                if let FieldDef::Padding(Some(PerBusInt::Array(sizes))) = &field.field_def {
                    if sizes.len() != layout.bus.len() {
                        return Err(Error::new(
                            sizes[0].span(),
                            format!(
                                "number of sizes ({}) does not match number of buses ({})",
                                sizes.len(),
                                layout.bus.len()
                            ),
                        ));
                    }
                }
            }
            layouts.push(layout);
        }
        Ok(Input {
            tock_registers,
            layouts,
        })
    }
}

impl Parse for Layout {
    fn parse(input: ParseStream) -> Result<Layout> {
        let (docs, bus) = layout_attributes(Attribute::parse_outer(input)?)?;
        Ok(Layout {
            docs,
            bus,
            visibility: input.parse()?,
            name: input.parse()?,
            value: input.parse()?,
        })
    }
}

/// Parses attributes that belong on a Layout. If no `#[bus]` or `#[buses(...)]` is specified,
/// returns an empty `BusAttr::Buses`. The attributes are returned in order (docs, buses).
fn layout_attributes(attributes: Vec<Attribute>) -> Result<(Vec<Attribute>, BusAttr)> {
    let mut docs = Vec::new();
    let mut bus: Option<Attribute> = None;
    for attr in attributes {
        match attr.path() {
            p if p.is_ident("doc") => docs.push(attr),
            p if p.is_ident("bus") || p.is_ident("buses") => {
                if let Some(prev_bus) = bus {
                    let mut error = Error::new_spanned(attr, "multiple bus attributes");
                    error.combine(Error::new_spanned(
                        prev_bus,
                        "note: bus already specified here",
                    ));
                    return Err(error);
                }
                bus = Some(attr);
            }
            p => return Err(Error::new(p.span(), "unknown attribute")),
        }
    }
    let bus = match bus {
        Some(bus) if bus.path().is_ident("bus") => BusAttr::Bus(bus.parse_args()?),
        Some(buses) => {
            let punctuated = buses.parse_args_with(Punctuated::<_, Token![,]>::parse_terminated)?;
            if punctuated.is_empty() {
                return Err(Error::new_spanned(buses, "buses list cannot be empty"));
            }
            BusAttr::Buses(punctuated.into_iter().collect())
        }
        None => BusAttr::Buses(Vec::new()),
    };
    Ok((docs, bus))
}

impl Parse for Value {
    fn parse(input: ParseStream) -> Result<Value> {
        // Distinguish between a single register and block by looking at the first token, which
        // should be either : (single) or { (block).
        if input.peek(Token![:]) {
            return input.parse().map(Value::Single);
        }
        if !input.peek(Brace) {
            return Err(input.error("expected one of: `:`, `{`"));
        }
        let entries;
        braced!(entries in input);
        let entries = Punctuated::<BlockEntry, Token![,]>::parse_terminated(&entries)?;
        let mut groups: Vec<Group> = Vec::new();
        let mut fields = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry {
                BlockEntry::Group(group) => {
                    if let Some(prev) = groups.iter().find(|g| g.name == group.name) {
                        let mut error = Error::new(group.name.span(), "duplicate group");
                        error.combine(Error::new(
                            prev.name.span(),
                            "note: group already declared here",
                        ));
                        return Err(error);
                    }
                    groups.push(group);
                }
                BlockEntry::Field(field) => fields.push(field),
            }
        }
        for field in &fields {
            if let FieldDef::Register {
                group: Some(group), ..
            } = &field.field_def
            {
                if !groups.iter().any(|g| g.name == *group) {
                    return Err(Error::new(group.span(), "undeclared group"));
                }
            }
        }
        for field in fields.iter().rev() {
            match field.field_def {
                FieldDef::Padding(None) => {
                    return Err(Error::new(
                        field.offsets[0].span(),
                        "last non-aliased field cannot be padding without a size",
                    ))
                }
                FieldDef::Register { aliased: true, .. } => continue,
                FieldDef::Padding(Some(_)) | FieldDef::Register { aliased: false, .. } => break,
            }
        }
        Ok(Value::Block { groups, fields })
    }
}

/// An entry in a register block's braces: either a field or a group declaration.
enum BlockEntry {
    Field(Field),
    Group(Group),
}

impl Parse for BlockEntry {
    fn parse(input: ParseStream) -> Result<BlockEntry> {
        let is_group_decl =
            |a: &Attribute| a.path().is_ident("group") && a.meta.require_list().is_ok();
        if !Attribute::parse_outer(&input.fork())?
            .iter()
            .any(is_group_decl)
        {
            return input.parse().map(BlockEntry::Field);
        }
        let mut docs = Vec::new();
        let mut name = None;
        for attr in Attribute::parse_outer(input)? {
            match attr.path() {
                p if p.is_ident("doc") => docs.push(attr),
                _ if name.is_none() && is_group_decl(&attr) => name = Some(attr.parse_args()?),
                p => return Err(Error::new(p.span(), "unknown attribute on group")),
            }
        }
        let name = name.unwrap();
        Ok(BlockEntry::Group(Group { docs, name }))
    }
}

impl Parse for Field {
    fn parse(input: ParseStream) -> Result<Field> {
        let mut aliased_attr: Option<Attribute> = None;
        let mut group_name: Option<Ident> = None;
        let mut doc_attrs = Vec::new();
        for attr in Attribute::parse_outer(input)? {
            match attr.path() {
                p if p.is_ident("doc") => doc_attrs.push(attr),
                p if p.is_ident("aliased") => {
                    if let Some(prev) = aliased_attr {
                        let mut error = Error::new_spanned(attr, "multiple #[aliased] attributes");
                        error.combine(Error::new_spanned(
                            prev,
                            "note: aliased already specified here",
                        ));
                        return Err(error);
                    }
                    let Meta::Path(_) = attr.meta else {
                        return Err(Error::new_spanned(attr, "#[aliased] cannot have arguments"));
                    };
                    aliased_attr = Some(attr);
                }
                p if p.is_ident("group") => {
                    let name = match &attr.meta {
                        Meta::NameValue(MetaNameValue {
                            value: Expr::Path(path),
                            ..
                        }) => path.path.get_ident(),
                        _ => None,
                    };
                    let Some(name) = name else {
                        return Err(Error::new_spanned(attr, "expected #[group = GroupName]"));
                    };
                    if let Some(prev) = group_name {
                        let mut error = Error::new(name.span(), "multiple #[group] attributes");
                        error.combine(Error::new(
                            prev.span(),
                            "note: group already specified here",
                        ));
                        return Err(error);
                    }
                    group_name = Some(name.clone());
                }
                p => return Err(Error::new(p.span(), "unknown attribute")),
            }
        }
        let offsets = input.parse()?;
        input.parse::<Token![=>]>()?;
        let mut field_def = input.parse()?;
        match field_def {
            FieldDef::Padding(_) => {
                if let Some(last) = doc_attrs.last() {
                    return Err(Error::new(last.span(), "padding cannot have doc comments"));
                }
                if let Some(aliased) = aliased_attr {
                    return Err(Error::new_spanned(aliased, "padding cannot be aliased"));
                }
                if let Some(group) = group_name {
                    return Err(Error::new(group.span(), "padding cannot be in a group"));
                }
            }
            FieldDef::Register {
                ref mut docs,
                ref mut aliased,
                ref mut group,
                ..
            } => {
                *docs = doc_attrs;
                *aliased = aliased_attr.is_some();
                *group = group_name;
            }
        }
        Ok(Field { offsets, field_def })
    }
}

impl Parse for FieldDef {
    fn parse(input: ParseStream) -> Result<FieldDef> {
        if input.peek(Token![_]) {
            // The underscore tells us this field is padding.
            input.parse::<Token![_]>()?;
            if !input.peek(Token![:]) {
                return Ok(FieldDef::Padding(None));
            }
            input.parse::<Token![:]>()?;
            return Ok(FieldDef::Padding(Some(input.parse()?)));
        }
        Ok(FieldDef::Register {
            docs: Vec::new(),
            aliased: false,
            group: None,
            name: input.parse()?,
            spec: input.parse()?,
        })
    }
}

impl Parse for PerBusInt {
    fn parse(input: ParseStream) -> Result<PerBusInt> {
        if !input.peek(Bracket) {
            return input.parse().map(PerBusInt::Single);
        }
        let contents;
        bracketed!(contents in input);
        let ints = Punctuated::<_, Token![,]>::parse_terminated(&contents)?;
        if ints.is_empty() {
            return Err(Error::new(contents.span(), "list cannot be empty"));
        }
        Ok(PerBusInt::Array(ints.into_iter().collect()))
    }
}

impl Parse for RegisterSpec {
    fn parse(input: ParseStream) -> Result<RegisterSpec> {
        input.parse::<Token![:]>()?;
        // Recursive function to parse the type specification (because syn makes it hard to consume
        // individual bracket tokens).
        fn parse_type(input: ParseStream, array_sizes: &mut Vec<LitInt>) -> Result<Type> {
            if !input.peek(Bracket) {
                return input.parse();
            }
            let inner;
            bracketed!(inner in input);
            let out = parse_type(&inner, array_sizes)?;
            inner.parse::<Token![;]>()?;
            array_sizes.push(inner.parse()?);
            Ok(out)
        }
        let mut array_sizes = Vec::new();
        let element_type = parse_type(input, &mut array_sizes)?;
        let operations = if input.peek(Brace) {
            let ops;
            braced!(ops in input);
            let ops = Punctuated::<_, Token![,]>::parse_terminated(&ops)?;
            Some(ops.into_iter().collect())
        } else {
            None
        };
        Ok(RegisterSpec {
            element_type,
            array_sizes,
            operations,
        })
    }
}
