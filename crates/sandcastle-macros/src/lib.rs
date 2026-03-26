use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, spanned::Spanned, FnArg, ItemTrait, Pat, ReturnType, TraitItem, Type,
};

/// Convert a PascalCase or camelCase identifier to snake_case.
fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

/// Proc macro attribute that transforms a trait into a sandcastle capability.
///
/// Given a trait with async methods, this generates:
/// - The original trait with `Send + Sync + 'static` supertraits
/// - A wrapper struct `{TraitName}Capability<T>` implementing `sandcastle::Capability`
/// - Dispatch logic that routes method calls by name, handling JSON (de)serialization
///
/// # Example
///
/// ```ignore
/// #[sandcastle::capability]
/// trait UserService {
///     async fn get_user(&self, id: u64) -> Result<User, ApiError>;
/// }
/// ```
#[proc_macro_attribute]
pub fn capability(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemTrait);
    match capability_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

struct MethodInfo {
    name: syn::Ident,
    params: Vec<(syn::Ident, Box<Type>)>,
}

fn capability_impl(mut trait_def: ItemTrait) -> syn::Result<proc_macro2::TokenStream> {
    let trait_name = trait_def.ident.clone();
    let capability_struct = format_ident!("{}Capability", trait_name);
    let snake_name = to_snake_case(&trait_name.to_string());

    // Ensure Send + Sync + 'static supertraits are present.
    ensure_supertrait(&mut trait_def, "Send");
    ensure_supertrait(&mut trait_def, "Sync");
    ensure_supertrait_lifetime(&mut trait_def, "static");

    // Parse all trait methods.
    let mut methods = Vec::new();
    for item in &trait_def.items {
        let method = match item {
            TraitItem::Fn(m) => m,
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "capability traits may only contain async fn methods",
                ));
            }
        };

        // Must be async.
        if method.sig.asyncness.is_none() {
            return Err(syn::Error::new(
                method.sig.span(),
                "capability methods must be async",
            ));
        }

        // Must have &self receiver.
        let has_self_ref = method.sig.inputs.first().is_some_and(|arg| match arg {
            FnArg::Receiver(r) => r.reference.is_some() && r.mutability.is_none(),
            _ => false,
        });
        if !has_self_ref {
            return Err(syn::Error::new(
                method.sig.span(),
                "capability methods must take &self as the first parameter",
            ));
        }

        // Must return Result<T, E>.
        match &method.sig.output {
            ReturnType::Default => {
                return Err(syn::Error::new(
                    method.sig.span(),
                    "capability methods must return Result<T, E>",
                ));
            }
            ReturnType::Type(_, ty) => {
                if !is_result_type(ty) {
                    return Err(syn::Error::new(
                        ty.span(),
                        "capability methods must return Result<T, E>",
                    ));
                }
            }
        }

        // Collect non-self parameters.
        let mut params = Vec::new();
        for arg in method.sig.inputs.iter().skip(1) {
            match arg {
                FnArg::Typed(pat_type) => {
                    let ident = match pat_type.pat.as_ref() {
                        Pat::Ident(pat_ident) => pat_ident.ident.clone(),
                        other => {
                            return Err(syn::Error::new(
                                other.span(),
                                "capability method parameters must be simple identifiers",
                            ));
                        }
                    };
                    params.push((ident, pat_type.ty.clone()));
                }
                FnArg::Receiver(_) => unreachable!(),
            }
        }

        methods.push(MethodInfo {
            name: method.sig.ident.clone(),
            params,
        });
    }

    // Generate method schema entries.
    let method_schemas = methods.iter().map(|m| {
        let name_str = m.name.to_string();

        // Build a JSON-Schema-style input schema describing the parameters.
        let property_entries = m.params.iter().map(|(ident, ty)| {
            let ident_str = ident.to_string();
            let ty_str = quote!(#ty).to_string();
            quote! {
                properties.insert(
                    ::std::string::String::from(#ident_str),
                    ::serde_json::json!({ "type": #ty_str }),
                );
                required.push(::serde_json::Value::String(::std::string::String::from(#ident_str)));
            }
        });

        let input_schema = if m.params.is_empty() {
            quote! { ::serde_json::json!({ "type": "object", "properties": {} }) }
        } else {
            quote! {{
                let mut properties = ::serde_json::Map::new();
                let mut required = ::std::vec::Vec::<::serde_json::Value>::new();
                #(#property_entries)*
                ::serde_json::json!({
                    "type": "object",
                    "properties": ::serde_json::Value::Object(properties),
                    "required": required,
                })
            }}
        };

        quote! {
            sandcastle::MethodSchema {
                name: ::std::string::String::from(#name_str),
                description: ::std::string::String::new(),
                input_schema: #input_schema,
                output_schema: ::serde_json::json!({}),
            }
        }
    });

    // Generate dispatch arms.
    let dispatch_arms = methods.iter().map(|m| {
        let method_name = &m.name;
        let method_str = method_name.to_string();
        let param_count = m.params.len();

        let call = if param_count == 0 {
            // No parameters besides &self.
            quote! {
                let result = self.inner.#method_name().await;
            }
        } else if param_count == 1 {
            // Single parameter: deserialize directly from the value, but also
            // accept an object with the parameter name as key.
            let (ident, ty) = &m.params[0];
            let ident_str = ident.to_string();
            quote! {
                let #ident: #ty = if input.is_object() {
                    match input.get(#ident_str) {
                        Some(v) => ::serde_json::from_value(v.clone()).map_err(|e| {
                            sandcastle::CapabilityError::Serialization(
                                ::std::format!("failed to deserialize parameter `{}`: {}", #ident_str, e)
                            )
                        })?,
                        None => ::serde_json::from_value(input).map_err(|e| {
                            sandcastle::CapabilityError::Serialization(
                                ::std::format!("failed to deserialize parameter `{}`: {}", #ident_str, e)
                            )
                        })?,
                    }
                } else {
                    ::serde_json::from_value(input).map_err(|e| {
                        sandcastle::CapabilityError::Serialization(
                            ::std::format!("failed to deserialize parameter `{}`: {}", #ident_str, e)
                        )
                    })?
                };
                let result = self.inner.#method_name(#ident).await;
            }
        } else {
            // Multiple parameters: deserialize from a JSON object.
            let extractions = m.params.iter().map(|(ident, ty)| {
                let ident_str = ident.to_string();
                quote! {
                    let #ident: #ty = {
                        let val = obj.get(#ident_str).ok_or_else(|| {
                            sandcastle::CapabilityError::Serialization(
                                ::std::format!("missing parameter `{}`", #ident_str)
                            )
                        })?;
                        ::serde_json::from_value(val.clone()).map_err(|e| {
                            sandcastle::CapabilityError::Serialization(
                                ::std::format!("failed to deserialize parameter `{}`: {}", #ident_str, e)
                            )
                        })?
                    };
                }
            });
            let param_idents = m.params.iter().map(|(ident, _)| ident);
            quote! {
                let obj = input.as_object().ok_or_else(|| {
                    sandcastle::CapabilityError::Serialization(
                        ::std::format!(
                            "expected JSON object for method `{}` with {} parameters",
                            #method_str,
                            #param_count
                        )
                    )
                })?;
                #(#extractions)*
                let result = self.inner.#method_name(#(#param_idents),*).await;
            }
        };

        quote! {
            #method_str => {
                #call
                match result {
                    Ok(val) => ::serde_json::to_value(val).map_err(|e| {
                        sandcastle::CapabilityError::Serialization(
                            ::std::format!("failed to serialize response: {}", e)
                        )
                    }),
                    Err(e) => Err(sandcastle::CapabilityError::InvocationFailed {
                        capability: ::std::string::String::from(#snake_name),
                        method: ::std::string::String::from(#method_str),
                        message: ::std::format!("{}", e),
                    }),
                }
            }
        }
    });

    let output = quote! {
        #trait_def

        /// Auto-generated capability wrapper for [`#trait_name`].
        pub struct #capability_struct<T> {
            inner: T,
        }

        impl<T> #capability_struct<T> {
            /// Wrap an implementation of [`#trait_name`] as a capability.
            pub fn new(inner: T) -> Self {
                Self { inner }
            }
        }

        #[async_trait::async_trait]
        impl<T> sandcastle::Capability for #capability_struct<T>
        where
            T: #trait_name + Send + Sync + 'static,
        {
            fn name(&self) -> &str {
                #snake_name
            }

            fn methods(&self) -> ::std::vec::Vec<sandcastle::MethodSchema> {
                ::std::vec![#(#method_schemas),*]
            }

            async fn call(
                &self,
                method: &str,
                input: ::serde_json::Value,
            ) -> ::std::result::Result<::serde_json::Value, sandcastle::CapabilityError> {
                match method {
                    #(#dispatch_arms)*
                    other => Err(sandcastle::CapabilityError::NotFound {
                        capability: ::std::string::String::from(#snake_name),
                        method: ::std::string::String::from(other),
                    }),
                }
            }
        }
    };

    Ok(output)
}

/// Check (heuristically) whether a type looks like `Result<_, _>`.
fn is_result_type(ty: &Type) -> bool {
    match ty {
        Type::Path(tp) => tp
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "Result"),
        _ => false,
    }
}

/// Ensure the trait has a given supertrait (by simple name).
fn ensure_supertrait(trait_def: &mut ItemTrait, name: &str) {
    let has = trait_def.supertraits.iter().any(|bound| match bound {
        syn::TypeParamBound::Trait(tb) => tb
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == name),
        _ => false,
    });
    if !has {
        let ident = format_ident!("{}", name);
        if !trait_def.supertraits.is_empty() {
            trait_def
                .supertraits
                .push_punct(syn::token::Plus::default());
        }
        trait_def
            .supertraits
            .push_value(syn::TypeParamBound::Trait(syn::TraitBound {
                paren_token: None,
                modifier: syn::TraitBoundModifier::None,
                lifetimes: None,
                path: syn::Path::from(ident),
            }));
    }
}

/// Ensure the trait has a `'static` lifetime supertrait.
fn ensure_supertrait_lifetime(trait_def: &mut ItemTrait, lt: &str) {
    let has = trait_def.supertraits.iter().any(|bound| match bound {
        syn::TypeParamBound::Lifetime(l) => l.ident == lt,
        _ => false,
    });
    if !has {
        if !trait_def.supertraits.is_empty() {
            trait_def
                .supertraits
                .push_punct(syn::token::Plus::default());
        }
        trait_def
            .supertraits
            .push_value(syn::TypeParamBound::Lifetime(syn::Lifetime::new(
                &format!("'{}", lt),
                proc_macro2::Span::call_site(),
            )));
    }
}
