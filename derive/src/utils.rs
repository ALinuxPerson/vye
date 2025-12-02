use darling::FromAttributes;
use syn::{
    Attribute, Block, Signature, Token, Type, Visibility, braced,
    parse::{Parse, ParseStream},
};

pub struct MaybeStubFn {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub sig: Signature,
    _semi_token: Option<Token![;]>,
    pub block: Option<Block>,
}

impl Parse for MaybeStubFn {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Parse standard parts: attributes, visibility, signature
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let sig: Signature = input.parse()?;

        // LOOKAHEAD: Check if the next token is a semicolon
        if input.peek(Token![;]) {
            let semi_token: Token![;] = input.parse()?;
            Ok(MaybeStubFn {
                attrs,
                vis,
                sig,
                _semi_token: Some(semi_token),
                block: None,
            })
        } else {
            // Otherwise, expect a standard block
            let block: Block = input.parse()?;
            Ok(MaybeStubFn {
                attrs,
                vis,
                sig,
                _semi_token: None,
                block: Some(block),
            })
        }
    }
}

pub struct InterfaceImpl {
    pub self_ty: Type,
    pub items: Vec<MaybeStubFn>,
}

impl Parse for InterfaceImpl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.call(Attribute::parse_outer)?;
        input.parse::<Token![impl]>()?;
        let self_ty: Type = input.parse()?;
        let content;
        braced!(content in input);

        let mut items = Vec::new();
        while !content.is_empty() {
            items.push(content.parse()?);
        }

        Ok(InterfaceImpl { self_ty, items })
    }
}

/// Parses attributes into type T, returning the parsed value and the remaining attributes
/// (excluding the ones consumed by T, marked by "vye").
pub fn extract_vye_attrs<T: FromAttributes>(
    attributes: &[Attribute],
) -> syn::Result<(Vec<&Attribute>, T)> {
    let value = T::from_attributes(attributes)?;
    let remaining_attributes = attributes
        .iter()
        .filter(|attr| !attr.path().is_ident("vye"))
        .collect();
    Ok((remaining_attributes, value))
}
