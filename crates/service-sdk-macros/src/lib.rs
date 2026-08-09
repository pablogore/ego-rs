//! Proc-macro attributes for the Service SDK: `#[service]`, `#[operation]`,
//! `#[authorize]`, `#[tenant_scoped]` (CORE-008A) and `#[idempotent]`
//! (PROD-012).

mod authorize;

/// Typed enum that unifies detection and stripping of SDK-internal attributes.
///
/// Adding a new SDK attribute requires only a new variant here — both detection
/// and stripping automatically pick it up via `SdkAttr::detect`.
#[derive(PartialEq, Clone, Copy)]
enum SdkAttr {
    Operation,
    Authorize,
    TenantScoped,
    Idempotent,
}

impl SdkAttr {
    fn detect(attr: &syn::Attribute) -> Option<Self> {
        if attr.path().is_ident("operation") {
            Some(Self::Operation)
        } else if attr.path().is_ident("authorize") {
            Some(Self::Authorize)
        } else if attr.path().is_ident("tenant_scoped") {
            Some(Self::TenantScoped)
        } else if attr.path().is_ident("idempotent") {
            Some(Self::Idempotent)
        } else {
            None
        }
    }
}

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{parse_macro_input, Ident, ItemFn, ItemStruct, ItemTrait, TraitItem};

#[derive(Debug)]
struct ServiceArgs {
    version: Option<String>,
    /// CORE-028 Stage 2B: names the trait a `#[service]` struct implements,
    /// linking it to that trait's resolution Tag (`impl_of = Trait` or
    /// `impl_of = crate::path::Trait`). `None` on trait annotations and on
    /// struct annotations with no trait link (unchanged pre-Stage-2B
    /// behavior).
    impl_of: Option<syn::Path>,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut version = None;
        let mut impl_of = None;
        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            if ident == "version" {
                if version.is_some() {
                    return Err(syn::Error::new(
                        ident.span(),
                        "#[service] duplicate 'version' argument",
                    ));
                }
                let version_lit: syn::LitStr = input.parse()?;
                version = Some(version_lit.value());
            } else if ident == "impl_of" {
                if impl_of.is_some() {
                    return Err(syn::Error::new(
                        ident.span(),
                        "#[service] duplicate 'impl_of' argument",
                    ));
                }
                let path: syn::Path = input.parse()?;
                impl_of = Some(path);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "unknown #[service] argument `{ident}` — expected `version` or `impl_of`"
                    ),
                ));
            }
            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(ServiceArgs { version, impl_of })
    }
}

/// Derives the resolution-Tag path from an `impl_of` trait path (CORE-028
/// Stage 2B, task 2.1/2.4): the ident portion is the final path segment plus
/// `Tag` (`Trait` -> `TraitTag`), while any module-path prefix is preserved
/// unchanged — the generated `Tag` type lives in the same module the trait
/// macro expanded it into, so a path-qualified `impl_of` must stay
/// path-qualified to resolve. The original `path` argument (used for the
/// `dyn Trait` coercion target) is never mutated by this function.
fn tag_path_from_impl_of(path: &syn::Path) -> syn::Path {
    let mut tag_path = path.clone();
    let last = tag_path
        .segments
        .last_mut()
        .expect("a parsed syn::Path always has at least one segment");
    last.ident = format_ident!("{}Tag", last.ident);
    tag_path
}

/// Declares a service contract on a trait (generates Tag, Ref, ServiceContract) or on a struct (generates Injectable).
#[proc_macro_attribute]
pub fn service(args: TokenStream, input: TokenStream) -> TokenStream {
    let service_args = parse_macro_input!(args as ServiceArgs);

    if let Ok(input_trait) = syn::parse::<ItemTrait>(input.clone()) {
        expand_service_trait(input_trait, service_args)
    } else if let Ok(input_struct) = syn::parse::<ItemStruct>(input.clone()) {
        expand_service_struct(input_struct, service_args)
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
    if service_args.impl_of.is_some() {
        let err = syn::Error::new(
            Span::call_site(),
            "#[service] `impl_of` is only valid on a struct annotation (links the struct to the trait's resolution Tag) — it has no effect on a trait annotation",
        )
        .to_compile_error();
        return TokenStream::from(err);
    }

    let trait_name = &input_trait.ident;
    let tag_name = Ident::new(&format!("{}Tag", trait_name), trait_name.span());
    let ref_name = Ident::new(&format!("{}Ref", trait_name), trait_name.span());

    let version_str = service_args.version.unwrap_or_else(|| "1.0.0".to_string());
    let parts: Vec<&str> = version_str.split('.').collect();
    // All three semver segments must parse as u32 — non-numeric versions are rejected with a spanned error.
    let bad_semver_err = || {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "#[service] version \"{version_str}\" is not valid semver — expected \"major.minor.patch\" with unsigned integers"
            ),
        )
        .to_compile_error()
    };
    let major: u32 = match parts.first().map(|s| s.parse::<u32>()) {
        Some(Ok(v)) => v,
        _ => return TokenStream::from(bad_semver_err()),
    };
    let minor: u32 = match parts.get(1).map(|s| s.parse::<u32>()) {
        Some(Ok(v)) => v,
        None => 0,
        Some(Err(_)) => return TokenStream::from(bad_semver_err()),
    };
    let patch: u32 = match parts.get(2).map(|s| s.parse::<u32>()) {
        Some(Ok(v)) => v,
        None => 0,
        Some(Err(_)) => return TokenStream::from(bad_semver_err()),
    };

    let mut operation_descriptors = Vec::new();
    let mut forwarding_methods = Vec::new();
    let mut output_items = Vec::new();

    for item in &input_trait.items {
        if let TraitItem::Fn(method) = item {
            let has_operation = method
                .attrs
                .iter()
                .any(|a| SdkAttr::detect(a) == Some(SdkAttr::Operation));

            let has_idempotent = method
                .attrs
                .iter()
                .any(|a| SdkAttr::detect(a) == Some(SdkAttr::Idempotent));

            if has_operation {
                let method_name = &method.sig.ident;

                // CORE-008A TASK-012: per-method opt-in classification (AD-007).
                let has_tenant_scoped = method
                    .attrs
                    .iter()
                    .any(|a| SdkAttr::detect(a) == Some(SdkAttr::TenantScoped));

                // Must detect and parse before stripping: consuming #[authorize] here prevents the E5 standalone sentinel from firing.
                let authorize_attr = method
                    .attrs
                    .iter()
                    .find(|a| SdkAttr::detect(a) == Some(SdkAttr::Authorize));

                let authorize_args_result: Option<syn::Result<(authorize::AuthorizeArgs, usize)>> =
                    authorize_attr.map(|attr| {
                        // Bare `#[authorize]` without argument list gets a custom spanned error before syn's generic one.
                        if !matches!(attr.meta, syn::Meta::List(_)) {
                            return Err(syn::Error::new_spanned(
                                &attr.meta,
                                "#[authorize] requires arguments: #[authorize(context = <ident>, permission = \"<resource>:<action>\")]",
                            ));
                        }
                        let tokens = attr.meta.require_list()?.tokens.clone();
                        let args = authorize::parse_authorize_args(tokens)?;
                        let idx = authorize::validate_context_ident_in_signature(
                            &args.context_ident,
                            &method.sig,
                        )?;
                        Ok::<(authorize::AuthorizeArgs, usize), syn::Error>((args, idx))
                    });

                let (maybe_authorize, maybe_ctx_idx) = match authorize_args_result {
                    Some(Err(e)) => {
                        return TokenStream::from(e.to_compile_error());
                    }
                    Some(Ok((args, idx))) => (Some(args), Some(idx)),
                    None => (None, None),
                };

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
                        idempotent: #has_idempotent,
                        mutating: true,
                    }
                });

                let return_type = &method.sig.output;

                // Index of the context parameter within arg_names/arg_types, if any (None
                // for the parameterless phantom-ctx case below). Needed both to build
                // inner_call_args (clone the ctx arg) and, as of CORE-008A TASK-009B, to
                // bind that same parameter as `mut` in the generated signature so
                // `enforce_tenant(&mut ctx_param)` can take a mutable borrow of it.
                let mut ctx_param_idx: Option<usize> = None;

                // Clone the first param (context) so the original stays alive for enforce_tenant and interceptor calls.
                let (ctx_param, inner_call_args): (proc_macro2::TokenStream, Vec<_>) = if arg_names
                    .is_empty()
                {
                    // Parameterless methods: ctx_param is a phantom token. Any method here
                    // without #[authorize] will get a compile error on enforce_tenant(&ctx) —
                    // that is intentional: all #[operation] methods must have a context parameter.
                    (quote! { ctx }, vec![])
                } else {
                    // When #[authorize] is present, derive ctx_param from context_ident so
                    // enforce_tenant and interceptors operate on the same variable as the guard.
                    // Use the index returned by validate_context_ident_in_signature
                    // directly — no second O(N) scan.
                    let (ctx, clone_idx) =
                        if let (Some(ref args), Some(idx)) = (&maybe_authorize, maybe_ctx_idx) {
                            let ci = &args.context_ident;
                            (quote! { #ci }, idx)
                        } else {
                            let first = &arg_names[0];
                            (quote! { #first }, 0)
                        };
                    ctx_param_idx = Some(clone_idx);
                    let call_args = arg_names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| {
                            if i == clone_idx {
                                quote! { #name.clone() }
                            } else {
                                quote! { #name }
                            }
                        })
                        .collect();
                    (ctx, call_args)
                };

                // #212 (PROD-003 follow-up): the context parameter is always rebound at
                // the top of the body via `with_operation_name` (see
                // `operation_name_binding` below). `enforce_tenant(&mut ctx_param)` takes
                // its `&mut` from that rebinding, so the incoming parameter itself is only
                // ever moved (never mutated in place) and must keep a plain binding — a
                // `mut` here would now be `unused_mut`. The `mut` that `#[tenant_scoped]`
                // needs lives on the rebinding instead (CORE-008A TASK-012).
                let sig_params: Vec<proc_macro2::TokenStream> = arg_names
                    .iter()
                    .zip(arg_types.iter())
                    .map(|(name, ty)| quote! { #name: #ty })
                    .collect();

                // #212 (PROD-003 follow-up): stamp the dispatched operation's name onto the
                // context once, right after it is bound and before any downstream consumer
                // (the #[authorize]/#[tenant_scoped] guards, on_request, the inner handler,
                // on_response/on_error) observes it — so the TracingInterceptor names the
                // request-boundary span after the operation. The context param is owned, so
                // consuming `with_operation_name` and rebinding is free; the rebinding is
                // `mut` only for #[tenant_scoped] operations, which take `&mut ctx_param`
                // for `enforce_tenant` (a `mut` on any other operation would be unused).
                // Skipped for the parameterless phantom-ctx case (no real context to stamp).
                let operation_name_binding = if ctx_param_idx.is_some() {
                    if has_tenant_scoped {
                        quote! { let mut #ctx_param = #ctx_param.with_operation_name(stringify!(#method_name)); }
                    } else {
                        quote! { let #ctx_param = #ctx_param.with_operation_name(stringify!(#method_name)); }
                    }
                } else {
                    quote! {}
                };

                // Authorization guards emit .await — only async methods are valid targets.
                if let Some(ref _args) = maybe_authorize {
                    if method.sig.asyncness.is_none() {
                        let err = syn::Error::new_spanned(
                            authorize_attr.unwrap(),
                            "#[authorize] can only be used on async methods",
                        );
                        return TokenStream::from(err.to_compile_error());
                    }
                }

                // Authorization guard — emitted at slot 1 only when #[authorize] is present.
                let authorize_guard = if let Some(ref args) = maybe_authorize {
                    let ctx_ident = &args.context_ident;
                    let resource_str = &args.resource;
                    let action_str = &args.action;

                    let error_type = match &method.sig.output {
                        syn::ReturnType::Type(_, ty) => result_error_type(ty).cloned(),
                        _ => None,
                    };

                    if let Some(err_ty) = error_type {
                        quote! {
                            // Const-closure assertion: evaluated at type-check time only, generates no code.
                            const _: fn() = || {
                                fn assert_from<E: From<ego_service_sdk::security::SecurityError>>() {}
                                assert_from::<#err_ty>();
                            };

                            // #[authorize] requires authentication. A missing SecurityContext
                            // is a request-level failure, not a disabled-security bypass.
                            //
                            // CORE-012A: the weak handle is upgraded *before* the
                            // MissingContext check (not after, as before this change)
                            // purely so a MissingContext denial can be recorded through
                            // it. This does NOT change existing error precedence:
                            // upgrading a dropped Weak just yields None here (it never
                            // errors), so MissingContext is still returned before
                            // ProviderError below when both conditions hold.
                            let __rt_opt = self.runtime.upgrade();
                            let __sec_ctx = #ctx_ident.security().ok_or_else(|| {
                                if let Some(__rt) = __rt_opt.as_ref() {
                                    __rt.record_security_denial(
                                        stringify!(#trait_name),
                                        stringify!(#method_name),
                                        ego_service_sdk::runtime::SecurityDenialKind::MissingContext,
                                    );
                                }
                                <#err_ty as From<ego_service_sdk::security::SecurityError>>::from(
                                    ego_service_sdk::security::SecurityError::MissingContext
                                )
                            })?;
                            let __rt = __rt_opt.ok_or_else(|| {
                                <#err_ty as From<ego_service_sdk::security::SecurityError>>::from(
                                    ego_service_sdk::security::SecurityError::ProviderError(
                                        "authorization provider unavailable: runtime dropped".into()
                                    )
                                )
                            })?;
                            let __provider = __rt.authorization_provider().ok_or_else(|| {
                                <#err_ty as From<ego_service_sdk::security::SecurityError>>::from(
                                    ego_service_sdk::security::SecurityError::CapabilityNotEnabled
                                )
                            })?;
                            ego_service_sdk::security::authorize_in_context(
                                Some(__sec_ctx),
                                ego_service_sdk::security::Resource {
                                    kind: std::borrow::Cow::Borrowed(#resource_str),
                                    id: None,
                                },
                                ego_service_sdk::security::Action(std::borrow::Cow::Borrowed(#action_str)),
                                __provider.as_ref(),
                            )
                            .await
                            .map_err(|e| {
                                if let Some(kind) = ego_service_sdk::runtime::SecurityDenialKind::from_security_error(&e) {
                                    __rt.record_security_denial(
                                        stringify!(#trait_name),
                                        stringify!(#method_name),
                                        kind,
                                    );
                                }
                                <#err_ty as From<ego_service_sdk::security::SecurityError>>::from(e)
                            })?;
                        }
                    } else {
                        let err = syn::Error::new(
                            method.sig.output.span(),
                            "#[authorize] requires the method return type to be written as `Result<_, E>` \
                             directly (type aliases are not supported in proc-macro context; \
                             expand the alias inline, e.g. `Result<MyResponse, MyError>`)",
                        );
                        return TokenStream::from(err.to_compile_error());
                    }
                } else {
                    quote! {}
                };

                // enforce_tenant is fallible (CORE-008A AD-009). A #[tenant_scoped]
                // operation gets the fallible `?` branch below (TASK-012); an unmarked
                // operation keeps today's best-effort call — the Result is discarded so
                // a resolution failure never surfaces, matching the pre-Phase-3 no-op
                // observable behavior (TASK-013 regression check).
                let enforce_tenant_block = if has_tenant_scoped {
                    let tenant_err_ty = match &method.sig.output {
                        syn::ReturnType::Type(_, ty) => result_error_type(ty).cloned(),
                        _ => None,
                    };

                    match tenant_err_ty {
                        Some(err_ty) => quote! {
                            // Const-closure assertion mirroring #[authorize]'s pattern
                            // (CORE-008A TASK-012): evaluated at type-check time only,
                            // generates no code.
                            const _: fn() = || {
                                fn assert_from<E: From<ego_service_sdk::security::SecurityError>>() {}
                                assert_from::<#err_ty>();
                            };

                            // Fallible enforcement — fails fast before the inner call
                            // (FR-009). A dropped runtime is itself an unresolvable
                            // context, not a disabled-tenancy bypass.
                            let __tenant_rt = self.runtime.upgrade().ok_or_else(|| {
                                <#err_ty as From<ego_service_sdk::security::SecurityError>>::from(
                                    ego_service_sdk::security::SecurityError::MissingContext
                                )
                            })?;
                            __tenant_rt
                                .enforce_tenant(&mut #ctx_param)
                                .map_err(|e| {
                                    if let Some(kind) = ego_service_sdk::runtime::SecurityDenialKind::from_security_error(&e) {
                                        __tenant_rt.record_security_denial(
                                            stringify!(#trait_name),
                                            stringify!(#method_name),
                                            kind,
                                        );
                                    }
                                    <#err_ty as From<ego_service_sdk::security::SecurityError>>::from(e)
                                })?;
                        },
                        None => {
                            let err = syn::Error::new(
                                method.sig.output.span(),
                                "#[tenant_scoped] requires the method return type to be written as `Result<_, E>` \
                                 directly (type aliases are not supported in proc-macro context; \
                                 expand the alias inline, e.g. `Result<MyResponse, MyError>`)",
                            );
                            return TokenStream::from(err.to_compile_error());
                        }
                    }
                } else {
                    // Unmarked operations do not call enforce_tenant at all (code-review
                    // fix, CORE-008A): the old best-effort call ran the real resolver and
                    // silently populated `ctx.canonical_tenant()` for authenticated
                    // requests even though the operation isn't tenant-scoped, and its
                    // Result was discarded regardless — real work for a value nobody
                    // consumes. Skipping the call entirely restores true "zero behavior
                    // change" for unmarked ops (TASK-013), matching the pre-Phase-3
                    // literal no-op.
                    quote! {}
                };

                forwarding_methods.push(quote! {
                    async fn #method_name(&self, #(#sig_params),*) #return_type {
                        #operation_name_binding
                        #authorize_guard
                        #enforce_tenant_block
                        let inner_ref = self.inner.clone();
                        let chain_ref = self.chain.clone();
                        let _ = chain_ref.on_request(&#ctx_param).await;
                        let result = inner_ref.#method_name(#(#inner_call_args),*).await;
                        match &result {
                            Ok(_) => { chain_ref.on_response(&#ctx_param).await.ok(); }
                            Err(e) => {
                                chain_ref
                                    .on_error(&#ctx_param, e as &dyn ego_service_sdk::error::ServiceErrorTrait)
                                    .await
                                    .ok();
                            }
                        }
                        result
                    }
                });

                let mut clean = method.clone();
                clean.attrs.retain(|a| SdkAttr::detect(a).is_none());
                output_items.push(TraitItem::Fn(clean));
            } else {
                // A #[tenant_scoped] method missing #[operation] must not fall through
                // unstripped: the standalone `tenant_scoped` attribute macro would later
                // fire its generic "must be inside a #[service] trait" error even though
                // it genuinely is inside one, masking the real problem. Catch it here
                // with a message that names the actual missing attribute.
                let has_tenant_scoped_without_operation = method
                    .attrs
                    .iter()
                    .any(|a| SdkAttr::detect(a) == Some(SdkAttr::TenantScoped));
                if has_tenant_scoped_without_operation {
                    let err = syn::Error::new_spanned(
                        &method.sig.ident,
                        "#[tenant_scoped] requires #[operation] on the same method",
                    );
                    return TokenStream::from(err.to_compile_error());
                }
                // Same reasoning for #[idempotent], and the stakes are higher: the
                // reservation slot this marker enables runs only in the generated
                // operation path, so on a method outside it the annotation records
                // an idempotency promise nothing reserves, replays or refuses.
                if has_idempotent {
                    let err = syn::Error::new_spanned(
                        &method.sig.ident,
                        "#[idempotent] requires #[operation] on the same method",
                    );
                    return TokenStream::from(err.to_compile_error());
                }
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

        impl ego_service_sdk::runtime::Resolvable for #tag_name {
            type Proxy = #ref_name;
            type Service = dyn #trait_name;

            fn create_proxy(
                inner: std::sync::Arc<dyn std::any::Any + std::marker::Send + std::marker::Sync>,
                chain: std::sync::Arc<ego_service_sdk::interceptor::InterceptorChain>,
                runtime: std::sync::Weak<ego_service_sdk::runtime::RuntimeInner>,
            ) -> Result<Self::Proxy, ego_service_sdk::runtime::RuntimeError> {
                let container = inner
                    .downcast::<ego_service_sdk::runtime::ResolvableContainer<dyn #trait_name>>()
                    .map_err(|_| ego_service_sdk::runtime::RuntimeError::ServiceNotFound {
                        type_name: std::any::type_name::<#tag_name>(),
                        required_by: None,
                    })?;
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

fn expand_service_struct(input_struct: ItemStruct, service_args: ServiceArgs) -> TokenStream {
    let struct_name = &input_struct.ident;

    let mut dep_keys: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut field_inits: Vec<proc_macro2::TokenStream> = Vec::new();

    if let syn::Fields::Named(fields) = &input_struct.fields {
        for field in &fields.named {
            let field_name = &field.ident;
            if let Some(dep_key) = classify_field_type(&field.ty) {
                dep_keys.push(dep_key);
            }
            if let Some(init_expr) = classify_field_init(&field.ty) {
                // DI field: resolve from RuntimeInner
                field_inits.push(quote! { #field_name: #init_expr });
            } else {
                // Plain field: use Default::default()
                field_inits.push(quote! { #field_name: Default::default() });
            }
        }
    }

    // CORE-028 Stage 2B (tasks 2.3/2.4): `impl_of = Trait` generates the
    // link from this struct to `Trait`'s resolution Tag, plus the concrete
    // `Arc<Self> -> Arc<dyn Trait>` coercion the marker trait's contract
    // requires. Absent, this is a no-op — a bare `#[service]` struct is
    // unaffected (spec.md "Bare `#[service]` struct usage is unaffected").
    let has_service_tag_impl = service_args.impl_of.map(|trait_path| {
        let tag_path = tag_path_from_impl_of(&trait_path);
        quote! {
            impl ego_service_sdk::runtime::HasServiceTag for #struct_name {
                type Tag = #tag_path;

                // Written as a literal `dyn Trait` (not the trait's abstract
                // `<Self::Tag as Resolvable>::Service` projection) — this is
                // what makes the coercion an ordinary concrete unsize
                // coercion rather than the invalid generic bound design.md's
                // E0405 spike ruled out. `<#tag_path as Resolvable>::Service`
                // normalizes to this exact `dyn #trait_path` at the trait's
                // own expansion site, so the impl still satisfies the
                // trait's abstract signature (design.md's associated-type
                // projection-equality decision).
                fn into_service(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn #trait_path> {
                    self
                }
            }
        }
    });

    let expanded = quote! {
        #input_struct

        impl ego_service_sdk::di::Injectable for #struct_name {
            fn dependencies() -> Vec<ego_service_sdk::di::DepKey>
            where Self: Sized {
                use std::any::TypeId;
                vec![#(#dep_keys),*]
            }

            fn build(rt: &ego_service_sdk::runtime::RuntimeInner) -> Result<Self, ego_service_sdk::runtime::RuntimeError>
            where Self: Sized {
                Ok(Self {
                    #(#field_inits),*
                })
            }
        }

        #has_service_tag_impl
    };

    TokenStream::from(expanded)
}

/// Returns the DI resolver expression for a field type (e.g. `rt.resolve_projection::<T>()?`); None for non-DI fields.
fn classify_field_init(ty: &syn::Type) -> Option<proc_macro2::TokenStream> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident_str = segment.ident.to_string();
            if let syn::PathArguments::AngleBracketed(ref args) = segment.arguments {
                if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                    match ident_str.as_str() {
                        "EntityRuntimeRef" => {
                            return Some(quote! {
                                rt.resolve_entity::<#inner_ty>()?
                            });
                        }
                        "ProjectionRef" => {
                            return Some(quote! {
                                rt.resolve_projection::<#inner_ty>()?
                            });
                        }
                        "AdapterRef" => {
                            return Some(quote! {
                                rt.resolve_adapter::<#inner_ty>()?
                            });
                        }
                        "ConfigValue" => {
                            return Some(quote! {
                                rt.resolve_config::<#inner_ty>()?
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

/// Maps a field type to a `DepKey` variant; returns `None` for non-DI fields.
fn classify_field_type(ty: &syn::Type) -> Option<proc_macro2::TokenStream> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident_str = segment.ident.to_string();
            if let syn::PathArguments::AngleBracketed(ref args) = segment.arguments {
                if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                    let variant = match ident_str.as_str() {
                        "EntityRuntimeRef" => "Entity",
                        "ProjectionRef" => "Projection",
                        "AdapterRef" => "Adapter",
                        "ConfigValue" => "Config",
                        _ => return None,
                    };
                    let variant = format_ident!("{}", variant);
                    return Some(quote! {
                        ego_service_sdk::di::DepKey::#variant(
                            std::any::TypeId::of::<#inner_ty>(),
                            std::any::type_name::<#inner_ty>()
                        )
                    });
                }
            }
        }
    }
    None
}

/// Shared helper — walks a `Result<_, E>` type node and returns the error type (second generic arg).
fn result_error_type(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if args.args.len() >= 2 {
                        if let syn::GenericArgument::Type(err_ty) = &args.args[1] {
                            return Some(err_ty);
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_error_types(ty: &syn::Type) -> proc_macro2::TokenStream {
    if let Some(syn::Type::Path(error_path)) = result_error_type(ty) {
        return quote! { vec![stringify!(#error_path).to_string()] };
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

/// Authorization marker for `#[service]` methods — rejected at compile time when used outside `#[service]`.
#[proc_macro_attribute]
pub fn authorize(_args: TokenStream, _input: TokenStream) -> TokenStream {
    let err = syn::Error::new(
        Span::call_site(),
        "#[authorize] can only be used on methods inside a #[service] trait",
    );
    TokenStream::from(err.to_compile_error())
}

/// Idempotency marker for `#[service]` operations (PROD-012) — rejected at
/// compile time when used outside `#[service]`.
///
/// Mirrors `#[authorize]` and `#[tenant_scoped]` rather than `#[operation]`, and
/// the reason is sharper here than for either: a marker that silently did
/// nothing when misapplied would leave an operation that everyone believes is
/// idempotent, with nothing reserving, replaying or refusing its retries. A
/// forgotten marker is a bug; a marker that looks present and is inert is a bug
/// nobody goes looking for.
#[proc_macro_attribute]
pub fn idempotent(_args: TokenStream, _input: TokenStream) -> TokenStream {
    let err = syn::Error::new(
        Span::call_site(),
        "#[idempotent] can only be used on methods inside a #[service] trait",
    );
    TokenStream::from(err.to_compile_error())
}

/// Tenant-enforcement classification marker for `#[service]` methods (CORE-008A
/// AD-007/TASK-012) — rejected at compile time when used outside `#[service]`.
///
/// Mirrors `#[authorize]`, not `#[operation]`: a `#[tenant_scoped]` marker that
/// silently did nothing when misapplied would be a false sense of enforcement,
/// exactly the fail-open risk AD-007 already flags for a forgotten marker.
/// Failing loudly here is cheaper than debugging a mistakenly-inert tenant guard.
#[proc_macro_attribute]
pub fn tenant_scoped(_args: TokenStream, _input: TokenStream) -> TokenStream {
    let err = syn::Error::new(
        Span::call_site(),
        "#[tenant_scoped] can only be used on methods inside a #[service] trait",
    );
    TokenStream::from(err.to_compile_error())
}
