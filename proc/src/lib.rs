#[proc_macro_derive(Asset, attributes(asset, serde))]
pub fn asset(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match parse(item).and_then(asset_impl) {
        Ok(tokens) => tokens,
        Err(error) => error.into_compile_error(),
    }
    .into()
}

#[proc_macro_derive(AssetField, attributes(asset, serde))]
pub fn asset_field(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match parse(item).and_then(asset_field_impl) {
        Ok(tokens) => tokens,
        Err(error) => error.into_compile_error(),
    }
    .into()
}

struct Parsed {
    complex: bool,
    derive_input: syn::DeriveInput,
    info: syn::Ident,
    futures: syn::Ident,
    decoded: syn::Ident,
    decode_error: syn::Ident,
    decode_field_errors: proc_macro2::TokenStream,
    build_error: syn::Ident,
    build_field_errors: proc_macro2::TokenStream,
    builder_bounds: proc_macro2::TokenStream,
    info_fields: proc_macro2::TokenStream,
    info_to_futures_fields: proc_macro2::TokenStream,
    futures_fields: proc_macro2::TokenStream,
    futures_to_decoded_fields: proc_macro2::TokenStream,
    decoded_fields: proc_macro2::TokenStream,
    decoded_to_asset_fields: proc_macro2::TokenStream,
    serde_attributes: Vec<syn::Attribute>,
    name: Option<syn::LitStr>,
}

fn parse(item: proc_macro::TokenStream) -> syn::Result<Parsed> {
    use syn::spanned::Spanned;

    let derive_input = syn::parse::<syn::DeriveInput>(item)?;

    let asset_attributes = derive_input
        .attrs
        .iter()
        .enumerate()
        .filter_map(|(index, attr)| {
            if attr.path.is_ident("asset") {
                Some(index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut name_arg = None;

    for idx in &asset_attributes {
        let attr = &derive_input.attrs[*idx];

        attr.parse_args_with(|stream: syn::parse::ParseStream| {
            match stream.parse::<syn::Ident>()? {
                i if i == "name" => {
                    let _eq = stream.parse::<syn::Token![=]>()?;

                    let name = stream.parse::<syn::LitStr>()?;
                    name_arg = Some(name);

                    if !stream.is_empty() {
                        return Err(syn::Error::new(stream.span(), "Expected end of arguments"));
                    }

                    Ok(())
                }
                i => Err(syn::Error::new_spanned(
                    i,
                    "Unexpected ident. Expected: 'name'",
                )),
            }
        })?;
    }

    let serde_attributes = derive_input
        .attrs
        .iter()
        .filter(|attr| attr.path.is_ident("serde"))
        .cloned()
        .collect();

    let mut decode_field_errors = proc_macro2::TokenStream::new();
    let mut build_field_errors = proc_macro2::TokenStream::new();
    let mut builder_bounds = proc_macro2::TokenStream::new();

    let info = quote::format_ident!("{}Info", derive_input.ident);
    let mut info_fields = proc_macro2::TokenStream::new();
    let mut info_to_futures_fields = proc_macro2::TokenStream::new();

    let futures = quote::format_ident!("{}Futures", derive_input.ident);
    let mut futures_fields = proc_macro2::TokenStream::new();
    let mut futures_to_decoded_fields = proc_macro2::TokenStream::new();

    let decoded = quote::format_ident!("{}Decoded", derive_input.ident);
    let mut decoded_fields = proc_macro2::TokenStream::new();
    let mut decoded_to_asset_fields = proc_macro2::TokenStream::new();

    let decode_error = quote::format_ident!("{}DecodeError", derive_input.ident);
    let build_error = quote::format_ident!("{}BuildError", derive_input.ident);

    let mut complex: bool = false;

    let data_struct = match &derive_input.data {
        syn::Data::Struct(data) => data,
        syn::Data::Enum(data) => {
            return Err(syn::Error::new_spanned(
                data.enum_token,
                "Only structs are currently supported by derive(Asset) macro",
            ))
        }
        syn::Data::Union(data) => {
            return Err(syn::Error::new_spanned(
                data.union_token,
                "Only structs are currently supported by derive(Asset) macro",
            ))
        }
    };

    for (index, field) in data_struct.fields.iter().enumerate() {
        let asset_attributes = field
            .attrs
            .iter()
            .enumerate()
            .filter_map(|(index, attr)| {
                if attr.path.is_ident("asset") {
                    Some(index)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let serde_attributes = field
            .attrs
            .iter()
            .filter(|attr| attr.path.is_ident("serde"));

        let ty = &field.ty;

        complex = true;

        let mut is_external = false;
        let mut as_type_arg = None;

        for idx in &asset_attributes {
            let attribute = &field.attrs[*idx];

            attribute.parse_args_with(|stream: syn::parse::ParseStream| {
                match stream.parse::<syn::Ident>()? {
                    i if i == "external" => {
                        if is_external {
                            return Err(syn::Error::new_spanned(
                                i,
                                "Attributes 'external' is already specified",
                            ));
                        }
                        is_external = true;

                        if !stream.is_empty() {
                            let args;
                            syn::parenthesized!(args in stream);
                            let _as = args.parse::<syn::Token![as]>()?;
                            let as_type = args.parse::<syn::Type>()?;
                            as_type_arg = Some(as_type);

                            if !stream.is_empty() {
                                return Err(syn::Error::new(
                                    stream.span(),
                                    "Expected end of arguments",
                                ));
                            }
                        }

                        Ok(())
                    }
                    i => Err(syn::Error::new_spanned(
                        i,
                        "Unexpected ident. Expected: 'external'",
                    )),
                }
            })?;
        }

        let as_type = as_type_arg.as_ref().unwrap_or(ty);

        let kind = match is_external {
            true => quote::quote!(::argosy::proc_macro::External),
            false => quote::quote!(::argosy::proc_macro::Inlined),
        };

        match &field.ident {
            Some(ident) => {
                let error_variant = quote::format_ident!("{}Error", snake_to_pascal(ident));
                let decode_error_text = syn::LitStr::new(
                    &format!("Failed to decode asset field '{ident}'. {{0}}"),
                    ident.span(),
                );
                let build_error_text = syn::LitStr::new(
                    &format!("Failed to build asset field '{ident}'. {{0}}"),
                    ident.span(),
                );

                decode_field_errors.extend(quote::quote!(
                    #[error(#decode_error_text)]
                    #error_variant(<#as_type as ::argosy::proc_macro::AssetField<#kind>>::DecodeError),
                ));
                build_field_errors.extend(quote::quote!(
                    #[error(#build_error_text)]
                    #error_variant(<#as_type as ::argosy::proc_macro::AssetField<#kind>>::BuildError),
                ));

                builder_bounds.extend(quote::quote!(
                    for<'build> ::argosy::proc_macro::FieldBuilder<'build, BuilderGenericParameter>: ::argosy::proc_macro::AssetFieldBuild<#kind, #as_type>,
                ));
                info_fields.extend(quote::quote!(
                    #(#serde_attributes)*
                    pub #ident: <#as_type as ::argosy::proc_macro::AssetField<#kind>>::Info,
                ));
                futures_fields.extend(quote::quote!(
                    pub #ident: <#as_type as ::argosy::proc_macro::AssetField<#kind>>::Fut,
                ));
                decoded_fields.extend(quote::quote!(
                    pub #ident: <#as_type as ::argosy::proc_macro::AssetField<#kind>>::Decoded,
                ));
                info_to_futures_fields.extend(quote::quote!(
                    #ident: <#as_type as ::argosy::proc_macro::AssetField<#kind>>::decode(info.#ident, loader),
                ));
                futures_to_decoded_fields.extend(quote::quote!(
                    #ident: futures.#ident.await.map_err(|err| #decode_error::#error_variant(err))?,
                ));
                decoded_to_asset_fields.extend(quote::quote!(
                    #ident: <#ty as ::argosy::proc_macro::From<#as_type>>::from(
                        <_ as ::argosy::proc_macro::AssetFieldBuild<#kind, #as_type>>::build(::argosy::proc_macro::FieldBuilder(builder), decoded.#ident)
                            .map_err(|err| #build_error::#error_variant(err))?
                    ),
                ));
            }
            None => {
                let error_variant = syn::Ident::new(&format!("Field{}Error", index), field.span());
                let decode_error_text = syn::LitStr::new(
                    &format!("Failed to decode asset field '{index}'. {{0}}"),
                    field.span(),
                );
                let build_error_text = syn::LitStr::new(
                    &format!("Failed to load asset field '{index}'. {{0}}"),
                    field.span(),
                );

                decode_field_errors.extend(quote::quote!(
                    #[error(#decode_error_text)]
                    #error_variant(<#as_type as ::argosy::proc_macro::AssetField<#kind>>::DecodeError),
                ));
                build_field_errors.extend(quote::quote!(
                    #[error(#build_error_text)]
                    #error_variant(<#as_type as ::argosy::proc_macro::AssetField<#kind>>::BuildError),
                ));

                builder_bounds.extend(quote::quote!(
                    for<'build> ::argosy::proc_macro::FieldBuilder<'build, BuilderGenericParameter>: ::argosy::proc_macro::AssetFieldBuild<#kind, #as_type>,
                ));
                info_fields.extend(quote::quote!(
                    #(#serde_attributes)*
                    pub <#as_type as ::argosy::proc_macro::AssetField<#kind>>::Info,
                ));
                futures_fields.extend(quote::quote!(
                    pub <#as_type as ::argosy::proc_macro::AssetField<#kind>>::Fut,
                ));
                decoded_fields.extend(quote::quote!(
                    pub <#as_type as ::argosy::proc_macro::AssetField<#kind>>::Decoded,
                ));
                info_to_futures_fields.extend(quote::quote!(
                    <#as_type as ::argosy::proc_macro::AssetField<#kind>>::decode(info.#index, loader),
                ));
                futures_to_decoded_fields.extend(quote::quote!(
                    futures.#index.await.map_err(|err| #decode_error::#error_variant(err))?,
                ));
                decoded_to_asset_fields.extend(quote::quote!(
                    <#ty as ::argosy::proc_macro::From<#as_type>>::from(
                        <_ as ::argosy::proc_macro::AssetFieldBuild<#kind, #as_type>>::build(::argosy::proc_macro::FieldBuilder(builder), decoded.#index)
                            .map_err(|err| #build_error::#error_variant(err))?
                    ),
                ));
            }
        }
    }

    Ok(Parsed {
        complex,
        derive_input,
        info,
        futures,
        decoded,
        decode_error,
        decode_field_errors,
        build_error,
        build_field_errors,
        builder_bounds,
        info_fields,
        info_to_futures_fields,
        futures_fields,
        futures_to_decoded_fields,
        decoded_fields,
        decoded_to_asset_fields,
        serde_attributes,
        name: name_arg,
    })
}

fn asset_impl(parsed: Parsed) -> syn::Result<proc_macro2::TokenStream> {
    let Parsed {
        complex,
        derive_input,
        info,
        futures,
        decoded,
        decode_error,
        build_error,
        decode_field_errors,
        build_field_errors,
        builder_bounds,
        info_fields,
        info_to_futures_fields,
        futures_fields,
        futures_to_decoded_fields,
        decoded_fields,
        decoded_to_asset_fields,
        serde_attributes,
        name,
    } = parsed;

    let name = match name {
        None => derive_input.ident.to_string(),
        Some(name) => name.value(),
    };

    let data_struct = match &derive_input.data {
        syn::Data::Struct(data) => data,
        _ => unreachable!(),
    };

    let ty = &derive_input.ident;

    let tokens = match data_struct.fields {
        syn::Fields::Unit => quote::quote! {
            #[derive(::argosy::proc_macro::Deserialize)]
            #(#serde_attributes)*
            pub struct #info;

            impl ::argosy::proc_macro::TrivialAsset for #ty {
                type Error = ::argosy::proc_macro::Infallible;

                fn name() -> &'static str {
                    #name
                }

                fn decode(bytes: ::argosy::proc_macro::Box<[u8]>) -> Result<Self, ::argosy::proc_macro::Infallible> {
                    ::argosy::proc_macro::Ok(#ty)
                }
            }

            impl ::argosy::proc_macro::AssetField<::argosy::proc_macro::Inlined> for #ty {
                type BuildError = ::argosy::proc_macro::Infallible;
                type DecodeError = ::argosy::proc_macro::Infallible;
                type Info = #info;
                type Decoded = Self;
                type Fut = ::argosy::proc_macro::Ready<::argosy::proc_macro::Result<Self, ::argosy::proc_macro::Infallible>>;

                fn decode(info: #info, _: &::argosy::proc_macro::Loader) -> Self::Fut {
                    use ::argosy::proc_macro::{ready, Ok};

                    ready(Ok(#ty))
                }
            }

            impl<BuilderGenericParameter> ::argosy::proc_macro::AssetFieldBuild<::argosy::proc_macro::Inlined, #ty> for ::argosy::proc_macro::FieldBuilder<'_, BuilderGenericParameter> {
                fn build(self, decoded: #ty) -> Result<#ty, ::argosy::proc_macro::Infallible> {
                    ::argosy::proc_macro::Ok(decoded)
                }
            }
        },
        syn::Fields::Unnamed(_) => todo!("Not yet implemented"),
        syn::Fields::Named(_) if complex => quote::quote! {
            #[derive(::argosy::proc_macro::Deserialize)]
            #(#serde_attributes)*
            pub struct #info { #info_fields }

            pub struct #futures { #futures_fields }

            pub struct #decoded { #decoded_fields }

            #[derive(::argosy::proc_macro::Debug, ::argosy::proc_macro::Error)]
            pub enum #decode_error {
                #[error("Failed to deserialize asset info. {0:#}")]
                Info(#[source]::argosy::proc_macro::DecodeError),

                #decode_field_errors
            }

            #[derive(::argosy::proc_macro::Debug, ::argosy::proc_macro::Error)]
            pub enum #build_error {
                #build_field_errors
            }

            impl ::argosy::proc_macro::Asset for #ty {
                type BuildError = #build_error;
                type DecodeError = #decode_error;
                type Decoded = #decoded;
                type Fut = ::argosy::proc_macro::BoxFuture<'static, ::argosy::proc_macro::Result<#decoded, #decode_error>>;

                fn name() -> &'static str {
                    #name
                }

                fn decode(bytes: ::argosy::proc_macro::Box<[u8]>, loader: &::argosy::proc_macro::Loader) -> Self::Fut {
                    use ::argosy::proc_macro::{DecodeError, Box, Result, Ok, Err};

                    let result: Result<#info, #decode_error> = ::argosy::proc_macro::deserialize_info(&*bytes).map_err(#decode_error::Info);

                    match result {
                        Ok(info) => {
                            let futures = #futures {
                                #info_to_futures_fields
                            };
                            Box::pin(async move {Ok(#decoded {
                                #futures_to_decoded_fields
                            })})
                        },
                        Err(err) => Box::pin(async move { Err(err) }),
                    }
                }
            }

            impl<BuilderGenericParameter> ::argosy::proc_macro::AssetBuild<BuilderGenericParameter> for #ty
            where
                #builder_bounds
            {
                fn build(builder: &mut BuilderGenericParameter, decoded: #decoded) -> ::argosy::proc_macro::Result<#ty, #build_error> {
                    ::argosy::proc_macro::Ok(#ty {
                        #decoded_to_asset_fields
                    })
                }
            }

            impl ::argosy::proc_macro::AssetField<::argosy::proc_macro::Inlined> for #ty {
                type BuildError = #build_error;
                type DecodeError = #decode_error;
                type Info = #info;
                type Decoded = #decoded;
                type Fut = ::argosy::proc_macro::BoxFuture<'static, Result<#decoded, #decode_error>>;

                fn decode(info: #info, loader: &::argosy::proc_macro::Loader) -> Self::Fut {
                    use ::argosy::proc_macro::{Box, Ok};

                    struct #futures { #futures_fields }

                    let futures = #futures {
                        #info_to_futures_fields
                    };

                    Box::pin(async move {Ok(#decoded {
                        #futures_to_decoded_fields
                    })})
                }
            }

            impl<BuilderGenericParameter> ::argosy::proc_macro::AssetFieldBuild<::argosy::proc_macro::Inlined, #ty> for ::argosy::proc_macro::FieldBuilder<'_, BuilderGenericParameter>
            where
                #builder_bounds
            {
                fn build(self, decoded: #decoded) -> ::argosy::proc_macro::Result<#ty, #build_error> {
                    let builder = self.0;
                    ::argosy::proc_macro::Ok(#ty {
                        #decoded_to_asset_fields
                    })
                }
            }
        },
        syn::Fields::Named(_) => quote::quote! {
            #[derive(::argosy::proc_macro::Deserialize)]
            #(#serde_attributes)*
            pub struct #info { #info_fields }

            impl ::argosy::proc_macro::TrivialAsset for #ty {
                type Error = ::argosy::proc_macro::DecodeError;

                fn name() -> &'static str {
                    #name
                }

                fn decode(bytes: ::argosy::proc_macro::Box<[u8]>) -> ::argosy::proc_macro::Result<Self, ::argosy::proc_macro::DecodeError> {
                    use ::argosy::proc_macro::{Ok, Err};

                    let decoded: #info = ::argosy::proc_macro::deserialize_info(&*bytes)?;

                    Ok(#ty {
                        #decoded_to_asset_fields
                    })
                }
            }

            impl ::argosy::proc_macro::AssetField<::argosy::proc_macro::Inlined> for #ty {
                type BuildError = ::argosy::proc_macro::Infallible;
                type DecodeError = ::argosy::proc_macro::Infallible;
                type Info = #info;
                type Decoded = Self;
                type Fut = ::argosy::proc_macro::Ready<::argosy::proc_macro::Result<Self, ::argosy::proc_macro::Infallible>>;

                fn decode(info: #info, _: &::argosy::proc_macro::Loader) -> Self::Fut {
                    use ::argosy::proc_macro::{ready, Ok};

                    let decoded = info;

                    ready(Ok(#ty {
                        #decoded_to_asset_fields
                    }))
                }
            }

            impl<BuilderGenericParameter> ::argosy::proc_macro::AssetFieldBuild<::argosy::proc_macro::Inlined, #ty> for ::argosy::proc_macro::FieldBuilder<'_, BuilderGenericParameter> {
                fn build(self, decoded: #ty) -> Result<#ty, ::argosy::proc_macro::Infallible> {
                    ::argosy::proc_macro::Ok(decoded)
                }
            }
        },
    };

    Ok(tokens)
}

fn asset_field_impl(parsed: Parsed) -> syn::Result<proc_macro2::TokenStream> {
    let Parsed {
        complex,
        derive_input,
        info,
        futures,
        decoded,
        decode_error,
        build_error,
        decode_field_errors,
        build_field_errors,
        builder_bounds,
        info_fields,
        info_to_futures_fields,
        futures_fields,
        futures_to_decoded_fields,
        decoded_fields,
        decoded_to_asset_fields,
        serde_attributes,
        name,
    } = parsed;

    if let Some(name) = name {
        return Err(syn::Error::new_spanned(
            name,
            "`derive(AssetField)` does not accept `asset(name = \"<name>\")` attribute",
        ));
    };

    let ty = &derive_input.ident;

    let data_struct = match &derive_input.data {
        syn::Data::Struct(data) => data,
        _ => unreachable!(),
    };

    let tokens = match data_struct.fields {
        syn::Fields::Unit => quote::quote! {
            #[derive(::argosy::proc_macro::Serialize, ::argosy::proc_macro::Deserialize)]
            #(#serde_attributes)*
            pub struct #info;

            impl ::argosy::proc_macro::AssetField<::argosy::proc_macro::Inlined> for #ty {
                type BuildError = ::argosy::proc_macro::Infallible;
                type DecodeError = ::argosy::proc_macro::Infallible;
                type Info = #info;
                type Decoded = Self;
                type Fut = ::argosy::proc_macro::Ready<::argosy::proc_macro::Result<Self, ::argosy::proc_macro::Infallible>>;

                fn decode(info: #info, _: &::argosy::proc_macro::Loader) -> Self::Fut {
                    use ::argosy::proc_macro::{ready, Ok};

                    ready(Ok(#ty))
                }
            }

            impl<BuilderGenericParameter> ::argosy::proc_macro::AssetFieldBuild<::argosy::proc_macro::Inlined, #ty> for ::argosy::proc_macro::FieldBuilder<'_, BuilderGenericParameter> {
                fn build(self, decoded: #ty) -> Result<#ty, ::argosy::proc_macro::Infallible> {
                    ::argosy::proc_macro::Ok(decoded)
                }
            }
        },

        syn::Fields::Unnamed(_) => todo!("Not yet implemented"),
        syn::Fields::Named(_) if complex => quote::quote! {
            #[derive(::argosy::proc_macro::Serialize, ::argosy::proc_macro::Deserialize)]
            #(#serde_attributes)*
            pub struct #info { #info_fields }

            pub struct #decoded { #decoded_fields }

            #[derive(::argosy::proc_macro::Debug, ::argosy::proc_macro::Error)]
            pub enum #decode_error {
                #decode_field_errors
            }

            #[derive(::argosy::proc_macro::Debug, ::argosy::proc_macro::Error)]
            pub enum #build_error {
                #build_field_errors
            }

            impl ::argosy::proc_macro::AssetField<::argosy::proc_macro::Inlined> for #ty {
                type BuildError = #build_error;
                type DecodeError = #decode_error;
                type Info = #info;
                type Decoded = #decoded;
                type Fut = ::argosy::proc_macro::BoxFuture<'static, Result<#decoded, #decode_error>>;

                fn decode(info: #info, loader: &::argosy::proc_macro::Loader) -> Self::Fut {
                    use ::argosy::proc_macro::{Box, Ok};

                    struct #futures { #futures_fields }

                    let futures = #futures {
                        #info_to_futures_fields
                    };

                    Box::pin(async move {Ok(#decoded {
                        #futures_to_decoded_fields
                    })})
                }
            }

            impl<BuilderGenericParameter> ::argosy::proc_macro::AssetFieldBuild<::argosy::proc_macro::Inlined, #ty> for ::argosy::proc_macro::FieldBuilder<'_, BuilderGenericParameter>
            where
                #builder_bounds
            {
                fn build(self, decoded: #decoded) -> ::argosy::proc_macro::Result<#ty, #build_error> {
                    let builder = self.0;
                    ::argosy::proc_macro::Ok(#ty {
                        #decoded_to_asset_fields
                    })
                }
            }
        },
        syn::Fields::Named(_) => quote::quote! {
            #[derive(::argosy::proc_macro::Serialize, ::argosy::proc_macro::Deserialize)]
            #(#serde_attributes)*
            pub struct #info { #info_fields }

            impl ::argosy::proc_macro::AssetField<::argosy::proc_macro::Inlined> for #ty {
                type BuildError = ::argosy::proc_macro::Infallible;
                type DecodeError = ::argosy::proc_macro::Infallible;
                type Info = #info;
                type Decoded = Self;
                type Fut = ::argosy::proc_macro::Ready<::argosy::proc_macro::Result<Self, ::argosy::proc_macro::Infallible>>;

                fn decode(info: #info, _: &::argosy::proc_macro::Loader) -> Self::Fut {
                    use ::argosy::proc_macro::{ready, Ok};

                    let decoded = info;

                    ready(Ok(#ty {
                        #decoded_to_asset_fields
                    }))
                }
            }

            impl<BuilderGenericParameter> ::argosy::proc_macro::AssetFieldBuild<::argosy::proc_macro::Inlined, #ty> for ::argosy::proc_macro::FieldBuilder<'_, BuilderGenericParameter> {
                fn build(self, decoded: #ty) -> Result<#ty, ::argosy::proc_macro::Infallible> {
                    ::argosy::proc_macro::Ok(decoded)
                }
            }
        },
    };

    Ok(tokens)
}

fn snake_to_pascal(input: &syn::Ident) -> syn::Ident {
    let mut result = String::new();
    let mut upper = true;
    for char in input.to_string().chars() {
        if char.is_ascii_alphabetic() {
            if upper {
                upper = false;
                result.extend(char.to_uppercase());
            } else {
                result.push(char);
            }
        } else if char.is_ascii_digit() {
            upper = true;
            result.push(char);
        } else if char == '_' {
            upper = true;
        } else {
            return input.clone();
        }
    }
    syn::Ident::new(&result, input.span())
}
