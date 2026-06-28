use syn::parse::Parser;

/// Parsed arguments from `#[authorize(context = <ident>, permission = "<resource>:<action>")]`.
///
/// This struct is populated by [`parse_authorize_args`] after all validation passes.
#[derive(Debug)]
pub(crate) struct AuthorizeArgs {
    /// The identifier of the context parameter (e.g., `ctx`).
    pub(crate) context_ident: syn::Ident,
    /// The resource portion of the permission literal (e.g., `"orders"`).
    pub(crate) resource: String,
    /// The action portion of the permission literal (e.g., `"read"`).
    pub(crate) action: String,
}

/// Parses and validates `#[authorize(context = <ident>, permission = "<resource>:<action>")]`.
///
/// Accepts exactly two named arguments: `context = <ident>` and
/// `permission = "<resource>:<action>"`. All errors are spanned at the
/// offending token via `syn::Error::new_spanned`.
///
/// The function calls `syn::meta::parser` internally to accumulate both
/// arguments before validating completeness, so it correctly detects missing
/// keys even when only one argument is supplied.
pub(crate) fn parse_authorize_args(
    tokens: proc_macro2::TokenStream,
) -> syn::Result<AuthorizeArgs> {
    let mut context_ident: Option<syn::Ident> = None;
    let mut permission_lit: Option<syn::LitStr> = None;

    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("context") {
            // context = <ident>
            let value = meta.value()?;
            if context_ident.is_some() {
                return Err(syn::Error::new_spanned(
                    &meta.path,
                    "#[authorize] duplicate 'context' argument",
                ));
            }
            // Peek for a string literal — that's an AD-4 non-ident error.
            if value.peek(syn::LitStr) {
                let lit: syn::LitStr = value.parse()?;
                return Err(syn::Error::new_spanned(
                    &lit,
                    "#[authorize] context must be a parameter name (identifier), not an expression",
                ));
            }
            let ident: syn::Ident = value.parse().map_err(|_| {
                syn::Error::new(meta.path.get_ident().unwrap().span(),
                    "#[authorize] context must be a parameter name (identifier), not an expression")
            })?;
            context_ident = Some(ident);
            Ok(())
        } else if meta.path.is_ident("permission") {
            // permission = "<resource>:<action>"
            let value = meta.value()?;
            if permission_lit.is_some() {
                return Err(syn::Error::new_spanned(
                    &meta.path,
                    "#[authorize] duplicate 'permission' argument",
                ));
            }
            // Non-literal is an AD-4 error — consume before returning so the buffer is drained.
            if !value.peek(syn::LitStr) {
                let span = value.cursor().token_stream();
                let _: proc_macro2::TokenStream = value.parse()?;
                return Err(syn::Error::new_spanned(
                    &span,
                    "#[authorize] permission must be a string literal known at compile time",
                ));
            }
            let lit: syn::LitStr = value.parse()?;
            permission_lit = Some(lit);
            Ok(())
        } else {
            let key = meta.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
            Err(syn::Error::new_spanned(
                &meta.path,
                format!("#[authorize] unknown argument '{key}'; expected 'context' and 'permission'"),
            ))
        }
    });

    parser.parse2(tokens)?;

    // After the full parse, check that both required args are present.
    match (context_ident, permission_lit) {
        (Some(ctx_ident), Some(perm_lit)) => {
            // Single split_once is sufficient: captures both halves and enforces the single-colon invariant atomically.
            let perm_value = perm_lit.value();
            let (resource, action) = match perm_value.split_once(':') {
                None => {
                    return Err(syn::Error::new_spanned(
                        &perm_lit,
                        format!(
                            "#[authorize] permission \"{perm_value}\" must have the form \"resource:action\""
                        ),
                    ));
                }
                Some((_, action)) if action.contains(':') => {
                    return Err(syn::Error::new_spanned(
                        &perm_lit,
                        format!(
                            "#[authorize] permission \"{perm_value}\" must have exactly one ':' (form \"resource:action\")"
                        ),
                    ));
                }
                Some((resource, action)) => (resource, action),
            };
            if resource.is_empty() {
                return Err(syn::Error::new_spanned(
                    &perm_lit,
                    format!(
                        "#[authorize] resource in \"{perm_value}\" must not be empty"
                    ),
                ));
            }
            if action.is_empty() {
                return Err(syn::Error::new_spanned(
                    &perm_lit,
                    format!(
                        "#[authorize] action in \"{perm_value}\" must not be empty"
                    ),
                ));
            }
            if resource == "*" || action == "*" {
                return Err(syn::Error::new_spanned(
                    &perm_lit,
                    format!(
                        "#[authorize] permission \"{perm_value}\" must not use wildcards; \
                         wildcards belong on the grant side, not the access-request side"
                    ),
                ));
            }

            Ok(AuthorizeArgs {
                context_ident: ctx_ident,
                resource: resource.to_string(),
                action: action.to_string(),
            })
        }
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[authorize] missing required argument; both 'context' and 'permission' are required",
        )),
    }
}

/// Validates that `ident` names a typed parameter present in `sig`.
///
/// Returns `Ok(usize)` — the 0-based index among typed parameters —
/// so the call site can locate the parameter without a second O(N) scan.
/// Returns `Err` (E6) spanned at `ident` when not found.
pub(crate) fn validate_context_ident_in_signature(
    ident: &syn::Ident,
    sig: &syn::Signature,
) -> syn::Result<usize> {
    let mut typed_idx: usize = 0;
    for fn_arg in &sig.inputs {
        if let syn::FnArg::Typed(pat_type) = fn_arg {
            if let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                if pat_ident.ident == *ident {
                    return Ok(typed_idx);
                }
            }
            typed_idx += 1;
        }
    }
    Err(syn::Error::new_spanned(
        ident,
        format!(
            "#[authorize] context parameter '{}' not found in method signature",
            ident
        ),
    ))
}

// ---------------------------------------------------------------------------
// Unit tests for parse_authorize_args and validate_context_ident_in_signature.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod authorize_args_tests {
    use super::*;

    // ── Valid input ───────────────────────────────────────────────────────────

    #[test]
    fn valid_input_parses_correctly() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = \"orders:read\"").expect("token parse");
        let args = parse_authorize_args(tokens).expect("expected Ok for valid input");
        assert_eq!(args.context_ident.to_string(), "ctx");
        assert_eq!(args.resource, "orders");
        assert_eq!(args.action, "read");
    }

    // ── E1: permission missing ':' ─────────────────────────────────────────

    #[test]
    fn e1_permission_missing_colon() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = \"ordersread\"").expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for E1");
        let msg = err.to_string();
        assert!(
            msg.contains("must have the form \"resource:action\""),
            "E1 message mismatch: {msg}"
        );
    }

    // ── E1b: permission has more than one ':' ─────────────────────────────

    #[test]
    fn e1b_permission_multiple_colons() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = \"a:b:c\"").expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for E1b");
        let msg = err.to_string();
        assert!(
            msg.contains("must have exactly one ':'"),
            "E1b message mismatch: {msg}"
        );
    }

    // ── E2: empty resource ────────────────────────────────────────────────

    #[test]
    fn e2_empty_resource() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = \":read\"").expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for E2");
        let msg = err.to_string();
        assert!(
            msg.contains("resource") && msg.contains("must not be empty"),
            "E2 message mismatch: {msg}"
        );
    }

    // ── E3: empty action ──────────────────────────────────────────────────

    #[test]
    fn e3_empty_action() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = \"orders:\"").expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for E3");
        let msg = err.to_string();
        assert!(
            msg.contains("action") && msg.contains("must not be empty"),
            "E3 message mismatch: {msg}"
        );
    }

    // ── E4: unknown named argument ────────────────────────────────────────

    #[test]
    fn e4_unknown_argument() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, perm = \"orders:read\"").expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for E4");
        let msg = err.to_string();
        assert!(
            msg.contains("unknown argument"),
            "E4 message mismatch: {msg}"
        );
    }

    // ── E4b: missing required argument ────────────────────────────────────

    #[test]
    fn e4b_missing_permission() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx").expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for E4b missing permission");
        let msg = err.to_string();
        assert!(
            msg.contains("missing required argument"),
            "E4b message mismatch: {msg}"
        );
    }

    #[test]
    fn e4b_missing_context() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("permission = \"orders:read\"").expect("token parse");
        let err =
            parse_authorize_args(tokens).expect_err("expected Err for E4b missing context");
        let msg = err.to_string();
        assert!(
            msg.contains("missing required argument"),
            "E4b message mismatch: {msg}"
        );
    }

    // ── AD-4 non-literal: permission is not a string literal ──────────────

    #[test]
    fn ad4_permission_non_literal() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = SOME_CONST").expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for AD-4 non-literal");
        let msg = err.to_string();
        assert!(
            msg.contains("must be a string literal"),
            "AD-4 non-literal message mismatch: {msg}"
        );
    }

    // ── AD-4 non-ident: context is not an identifier ──────────────────────

    #[test]
    fn ad4_context_non_ident() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = \"not_an_ident\", permission = \"orders:read\"")
                .expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for AD-4 non-ident");
        let msg = err.to_string();
        assert!(
            msg.contains("must be a parameter name") || msg.contains("identifier"),
            "AD-4 non-ident message mismatch: {msg}"
        );
    }

    // ── S3: duplicate keys ────────────────────────────────────────────────

    #[test]
    fn e_duplicate_context() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, context = other, permission = \"orders:read\"")
                .expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for duplicate context");
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate 'context' argument"),
            "duplicate context message mismatch: {msg}"
        );
    }

    #[test]
    fn e_duplicate_permission() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = \"orders:read\", permission = \"orders:write\"")
                .expect("token parse");
        let err = parse_authorize_args(tokens).expect_err("expected Err for duplicate permission");
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate 'permission' argument"),
            "duplicate permission message mismatch: {msg}"
        );
    }

    // ── AC-3.6: valid 'resource:action' does NOT trigger E2 ──────────────

    #[test]
    fn ac3_6_valid_nonempty_resource_does_not_trigger_e2() {
        let tokens: proc_macro2::TokenStream =
            syn::parse_str("context = ctx, permission = \"orders:read\"").expect("token parse");
        let args = parse_authorize_args(tokens).expect("valid input must not trigger E2");
        assert_eq!(args.resource, "orders");
    }

    // ── validate_context_ident_in_signature ──────────────────────────────

    #[test]
    fn validate_context_ident_found_in_signature() {
        let sig: syn::Signature =
            syn::parse_str("async fn foo(&self, ctx: ServiceContext) -> Result<(), E>")
                .expect("sig parse");
        let ident: syn::Ident = syn::parse_str("ctx").expect("ident parse");
        // Returns the 0-based typed-param index so callers avoid re-scanning the input list.
        let idx = validate_context_ident_in_signature(&ident, &sig)
            .expect("ctx is the first typed param");
        assert_eq!(idx, 0, "ctx is typed-param index 0 (self is skipped)");
    }

    #[test]
    fn validate_context_ident_not_found_emits_e6() {
        let sig: syn::Signature =
            syn::parse_str("async fn foo(&self, ctx: ServiceContext) -> Result<(), E>")
                .expect("sig parse");
        let ident: syn::Ident = syn::parse_str("wrong").expect("ident parse");
        let err =
            validate_context_ident_in_signature(&ident, &sig).expect_err("expected E6 error");
        let msg = err.to_string();
        assert!(
            msg.contains("context parameter 'wrong' not found"),
            "E6 message mismatch: {msg}"
        );
    }
}
