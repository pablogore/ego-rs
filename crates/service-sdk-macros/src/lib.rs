//! Proc-macro attributes for the Service SDK: `#[service]` and `#[operation]`.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Ident, ItemFn, ItemStruct, ItemTrait, TraitItem};

#[derive(Debug)]
struct ServiceArgs {
    version: Option<String>,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut version = None;
        if input.peek(syn::Ident) {
            let ident: syn::Ident = input.parse()?;
            if ident == "version" {
                input.parse::<syn::Token![=]>()?;
                let version_lit: syn::LitStr = input.parse()?;
                version = Some(version_lit.value());
            }
        }
        Ok(ServiceArgs { version })
    }
}

/// Declares a service contract on a trait or struct.
///
/// On a trait: emits `{TraitName}Tag` (registry key ZST), `{TraitName}Ref` (proxy), and `ServiceContract`.
/// On a struct: emits `Injectable` with `dependencies()` for `ProjectionRef`, `AdapterRef`, `ConfigValue` fields.
#[proc_macro_attribute]
pub fn service(args: TokenStream, input: TokenStream) -> TokenStream {
    let service_args = parse_macro_input!(args as ServiceArgs);

    if let Ok(input_trait) = syn::parse::<ItemTrait>(input.clone()) {
        expand_service_trait(input_trait, service_args)
    } else if let Ok(input_struct) = syn::parse::<ItemStruct>(input.clone()) {
        expand_service_struct(input_struct)
    } else {
        let err = syn::Error::new(
            Span::call_site(),
            "#[service] can only be applied to a `trait` or a `struct`",
        )
        .to_compile_error();
        TokenStream::from(err)
    }
}

fn expand_service_trait(input_trait: ItemTrait, service_args: ServiceArgs) -> TokenStream {
    let trait_name = &input_trait.ident;
    let tag_name = Ident::new(&format!("{}Tag", trait_name), trait_name.span());
    let ref_name = Ident::new(&format!("{}Ref", trait_name), trait_name.span());

    let version_str = service_args.version.unwrap_or_else(|| "1.0.0".to_string());
    let parts: Vec<&str> = version_str.split('.').collect();
    let major = parts
        .first()
        .map(|s| s.parse::<u32>().unwrap_or(1))
        .unwrap_or(1);
    let minor = parts
        .get(1)
        .map(|s| s.parse::<u32>().unwrap_or(0))
        .unwrap_or(0);
    let patch = parts
        .get(2)
        .map(|s| s.parse::<u32>().unwrap_or(0))
        .unwrap_or(0);

    let mut operation_descriptors = Vec::new();
    let mut forwarding_methods = Vec::new();
    let mut output_items = Vec::new();

    for item in &input_trait.items {
        if let TraitItem::Fn(method) = item {
            let has_operation = method.attrs.iter().any(|a| a.path().is_ident("operation"));
            if has_operation {
                let method_name = &method.sig.ident;

                let mut input_types = Vec::new();
                let mut arg_names: Vec<proc_macro2::TokenStream> = Vec::new();
                let mut arg_types: Vec<proc_macro2::TokenStream> = Vec::new();
                for fn_input in method.sig.inputs.iter() {
                    if let syn::FnArg::Typed(pat_type) = fn_input {
                        let ty = &pat_type.ty;
                        let pat = &pat_type.pat;
                        input_types.push(quote! { stringify!(#ty).to_string() });
                        arg_names.push(quote! { #pat });
                        arg_types.push(quote! { #ty });
                    }
                }

                let (output_type_str, error_types_ts) = match &method.sig.output {
                    syn::ReturnType::Type(_, ty) => {
                        let out_str = quote! { stringify!(#ty) };
                        let errs = extract_error_types(ty);
                        (out_str, errs)
                    }
                    _ => (quote! { "()" }, quote! { vec![] }),
                };

                operation_descriptors.push(quote! {
                    ego_service_sdk::contract::OperationDescriptor {
                        name: stringify!(#method_name).to_string(),
                        input: vec![#(#input_types),*],
                        output: #output_type_str.to_string(),
                        errors: #error_types_ts,
                        description: None,
                        metadata: std::collections::HashMap::new(),
                        idempotent: false,
                        mutating: true,
                    }
                });

                let return_type = &method.sig.output;
                forwarding_methods.push(quote! {
                    async fn #method_name(&self, #(#arg_names: #arg_types),*) #return_type {
                        let ctx = ego_service_sdk::context::ServiceContext::current()
                            .unwrap_or_default();
                        if let Some(rt) = self.runtime.upgrade() {
                            rt.enforce_tenant(&ctx);
                        }
                        let inner_ref = self.inner.clone();
                        let chain_ref = self.chain.clone();
                        // ctx_for_scope moves into the closure; inner_ctx is re-read inside
                        // so the task-local is active when the impl body runs.
                        let ctx_for_scope = ctx.clone();
                        ctx_for_scope.scope(|| async move {
                            let inner_ctx = ego_service_sdk::context::ServiceContext::current()
                                .unwrap_or_default();
                            let _ = chain_ref.on_request(&inner_ctx).await;
                            match inner_ref.#method_name(#(#arg_names),*).await {
                                Ok(v) => {
                                    chain_ref.on_response(&inner_ctx).await.ok();
                                    Ok(v)
                                }
                                Err(e) => {
                                    chain_ref
                                        .on_error(
                                            &inner_ctx,
                                            &e as &dyn ego_service_sdk::error::ServiceErrorTrait,
                                        )
                                        .await
                                        .ok();
                                    Err(e)
                                }
                            }
                        }).await
                    }
                });

                let mut clean = method.clone();
                clean.attrs.retain(|a| !a.path().is_ident("operation"));
                output_items.push(TraitItem::Fn(clean));
            } else {
                output_items.push(item.clone());
            }
        } else {
            output_items.push(item.clone());
        }
    }

    // Arc<dyn TraitName> requires Send + Sync supertraits.
    let mut output_trait = ItemTrait {
        items: output_items,
        ..input_trait.clone()
    };
    let send_bound: syn::TypeParamBound = syn::parse_quote!(Send);
    let sync_bound: syn::TypeParamBound = syn::parse_quote!(Sync);
    output_trait.supertraits.push(send_bound);
    output_trait.supertraits.push(sync_bound);

    let expanded = quote! {
        #[ego_service_sdk::async_trait::async_trait]
        #output_trait

        /// Zero-sized type used as the registry key for `#trait_name`.
        pub struct #tag_name;

        /// Generated proxy for `#trait_name`.
        pub struct #ref_name {
            inner: std::sync::Arc<dyn #trait_name>,
            chain: std::sync::Arc<ego_service_sdk::interceptor::InterceptorChain>,
            runtime: std::sync::Weak<ego_service_sdk::runtime::RuntimeInner>,
        }

        impl #ref_name {
            pub fn new(
                inner: std::sync::Arc<dyn #trait_name>,
                chain: std::sync::Arc<ego_service_sdk::interceptor::InterceptorChain>,
                runtime: std::sync::Weak<ego_service_sdk::runtime::RuntimeInner>,
            ) -> Self {
                Self { inner, chain, runtime }
            }
        }

        #[ego_service_sdk::async_trait::async_trait]
        impl #trait_name for #ref_name {
            #(#forwarding_methods)*
        }

            // Tag impl — enables runtime resolution.
        impl ego_service_sdk::runtime::Resolvable for #tag_name {
            type Proxy = #ref_name;

            fn create_proxy(
                inner: std::sync::Arc<dyn std::any::Any + std::marker::Send + std::marker::Sync>,
                chain: std::sync::Arc<ego_service_sdk::interceptor::InterceptorChain>,
                runtime: std::sync::Weak<ego_service_sdk::runtime::RuntimeInner>,
            ) -> Result<Self::Proxy, ego_service_sdk::runtime::RuntimeError> {
                let container = inner
                    .downcast::<ego_service_sdk::runtime::ResolvableContainer<dyn #trait_name>>()
                    .map_err(|_| ego_service_sdk::runtime::RuntimeError::DependencyNotFound)?;
                Ok(#ref_name::new(container.0.clone(), chain, runtime))
            }
        }

        // Tag impl avoids orphan rule violations from a blanket impl.
        impl ego_service_sdk::contract::ServiceContract for #tag_name {
            fn type_id() -> &'static str {
                std::any::type_name::<Self>()
            }

            fn name() -> &'static str {
                stringify!(#trait_name)
            }

            fn version() -> ego_service_sdk::contract::ContractVersion {
                ego_service_sdk::contract::ContractVersion::new(#major, #minor, #patch)
            }

            fn descriptor() -> ego_service_sdk::contract::ServiceDescriptor {
                ego_service_sdk::contract::ServiceDescriptor {
                    name: stringify!(#trait_name).to_string(),
                    version: <Self as ego_service_sdk::contract::ServiceContract>::version(),
                    operations: vec![#(#operation_descriptors),*],
                    description: None,
                    metadata: std::collections::HashMap::new(),
                }
            }

            fn operations() -> Vec<ego_service_sdk::contract::OperationDescriptor> {
                <Self as ego_service_sdk::contract::ServiceContract>::descriptor().operations
            }
        }
    };

    TokenStream::from(expanded)
}

fn expand_service_struct(input_struct: ItemStruct) -> TokenStream {
    let struct_name = &input_struct.ident;

    let mut dep_keys: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut has_di_fields = false;
    let mut plain_field_inits: Vec<proc_macro2::TokenStream> = Vec::new();

    if let syn::Fields::Named(fields) = &input_struct.fields {
        for field in &fields.named {
            let field_name = &field.ident;
            if let Some(dep_key) = classify_field_type(&field.ty) {
                dep_keys.push(dep_key);
                has_di_fields = true;
            } else {
                plain_field_inits.push(quote! { #field_name: Default::default() });
            }
        }
    }

    // Structs with DI fields return DependencyNotFound until RuntimeInner wires real resolvers.
    // Structs with only plain fields can be built immediately via Default.
    let build_body = if has_di_fields {
        quote! {
            Err(ego_service_sdk::runtime::RuntimeError::DependencyNotFound)
        }
    } else {
        quote! {
            Ok(Self { #(#plain_field_inits),* })
        }
    };

    let expanded = quote! {
        #input_struct

        impl ego_service_sdk::di::Injectable for #struct_name {
            fn dependencies() -> Vec<ego_service_sdk::di::DepKey>
            where Self: Sized {
                use std::any::TypeId;
                vec![#(#dep_keys),*]
            }

            fn build(_rt: &ego_service_sdk::runtime::RuntimeInner) -> Result<Self, ego_service_sdk::runtime::RuntimeError>
            where Self: Sized {
                #build_body
            }
        }
    };

    TokenStream::from(expanded)
}

/// Maps a field type to a `DepKey` variant; returns `None` for non-DI fields.
/// `EntityRef<T>` is excluded — it is owned by entity-sdk (INV-008).
fn classify_field_type(ty: &syn::Type) -> Option<proc_macro2::TokenStream> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident_str = segment.ident.to_string();
            if let syn::PathArguments::AngleBracketed(ref args) = segment.arguments {
                if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                    match ident_str.as_str() {
                        "ProjectionRef" => {
                            return Some(quote! {
                                ego_service_sdk::di::DepKey::Projection(
                                    std::any::TypeId::of::<#inner_ty>()
                                )
                            });
                        }
                        "AdapterRef" => {
                            return Some(quote! {
                                ego_service_sdk::di::DepKey::Adapter(
                                    std::any::TypeId::of::<#inner_ty>()
                                )
                            });
                        }
                        "ConfigValue" => {
                            return Some(quote! {
                                ego_service_sdk::di::DepKey::Config(
                                    std::any::TypeId::of::<#inner_ty>()
                                )
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    None
}

fn extract_error_types(ty: &syn::Type) -> proc_macro2::TokenStream {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if args.args.len() >= 2 {
                        if let syn::GenericArgument::Type(syn::Type::Path(error_path)) =
                            &args.args[1]
                        {
                            return quote! { vec![stringify!(#error_path).to_string()] };
                        }
                    }
                }
            }
        }
    }
    quote! { vec![] }
}

#[cfg(test)]
mod tests;

/// Marker attribute consumed by `#[service]` to identify operation methods; passes through unchanged.
#[proc_macro_attribute]
pub fn operation(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);
    TokenStream::from(quote! { #input_fn })
}
