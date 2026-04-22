//! Proc macros for `jackdaw_api`.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    Expr, ExprLit, ExprPath, ItemFn, Lit, LitBool, LitStr, MetaNameValue, Path, Token, Visibility,
    parse_macro_input, punctuated::Punctuated, spanned::Spanned,
};

/// Marks a plain Bevy system function as an operator. Generates the
/// zero-sized action type, the `InputAction` derive, and the
/// `Operator` trait impl, leaving the function itself in place as
/// the `execute` system.
///
/// Required keys:
/// - `id`: the global operator id string
/// - `label`: human-readable label
///
/// Optional keys:
/// - `description`: long-form description (default `""`)
/// - `modal`: `bool`, default `false`
/// - `allows_undo`: `bool`, default `true`. When `false`, this operator never
///   creates an undo history entry.
/// - `is_available`: path to a Bevy system returning `bool` that
///   decides whether the operator can run in the current editor
///   state. Runs before the execute system on every `World::operator`
///   call and via `World::is_operator_available`. If it returns
///   `false`, the operator returns an error without executing.
/// - `cancel`: path to a Bevy system invoked when the operator is
///   cancelled.
/// - `name`: override the generated struct name. Default is
///   `PascalCase(fn_name) + "Op"`.
///
/// ```rust,ignore
/// use jackdaw_api::prelude::*;
///
/// fn time_is_running(time: Res<Time>) -> bool {
///     time.delta_secs_f32() > 0.0
/// }
///
/// #[operator(id = "sample.hello", label = "Hello", is_available = time_is_running)]
/// fn hello(_: In<OperatorParameters>) -> OperatorResult {
///     info!("hello");
///     OperatorResult::Finished
/// }
/// ```
///
/// Expands to a `HelloOp` struct with `InputAction` derived and an
/// `impl Operator for HelloOp` whose `register_execute` registers the
/// `hello` function as a Bevy system. When `is_available` is given,
/// `register_availability_check` is emitted too.
#[proc_macro_attribute]
pub fn operator(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(
        attr with Punctuated::<MetaNameValue, Token![,]>::parse_terminated
    );
    let mut item_fn = parse_macro_input!(item as ItemFn);

    let mut id: Option<Expr> = None;
    let mut label: Option<Expr> = None;
    let mut description: Option<Expr> = None;
    let mut modal: bool = false;
    let mut allows_undo: bool = true;
    let mut name_override: Option<String> = None;
    let mut is_available: Option<Path> = None;
    let mut cancel: Option<Path> = None;

    for arg in args {
        let Some(key) = arg.path.get_ident().map(|i| i.to_string()) else {
            continue;
        };
        match key.as_str() {
            "id" => {
                if let Some(s) = as_str_expr(&arg.value) {
                    id = Some(s);
                }
            }
            "label" => {
                if let Some(s) = as_str_expr(&arg.value) {
                    label = Some(s);
                }
            }
            "description" => {
                if let Some(s) = as_str_expr(&arg.value) {
                    description = Some(s);
                }
            }
            "modal" => {
                if let Some(b) = as_lit_bool(&arg.value) {
                    modal = b.value;
                }
            }
            "allows_undo" => {
                if let Some(b) = as_lit_bool(&arg.value) {
                    allows_undo = b.value;
                }
            }
            "name" => {
                if let Some(s) = as_lit_str(&arg.value) {
                    name_override = Some(s.value());
                }
            }
            "is_available" => {
                if let Some(p) = as_path(&arg.value) {
                    is_available = Some(p);
                } else {
                    return syn::Error::new(
                        arg.value.span(),
                        "`is_available` must be the path of a Bevy system returning `bool`",
                    )
                    .into_compile_error()
                    .into();
                }
            }
            "cancel" => {
                if let Some(p) = as_path(&arg.value) {
                    cancel = Some(p);
                } else {
                    return syn::Error::new(
                        arg.value.span(),
                        "`cancel` must be the path of a Bevy system",
                    )
                    .into_compile_error()
                    .into();
                }
            }
            other => {
                return syn::Error::new(
                    arg.path.span(),
                    format!("unknown `#[operator]` argument: `{other}`"),
                )
                .into_compile_error()
                .into();
            }
        }
    }

    let Some(id) = id else {
        return syn::Error::new(Span::call_site(), "`#[operator]` requires `id = \"...\"`")
            .into_compile_error()
            .into();
    };
    let label = label.unwrap_or(id.clone());
    let description = description.unwrap_or_else(|| {
        Expr::Lit(ExprLit {
            lit: Lit::Str(LitStr::new("", Span::call_site())),
            attrs: vec![],
        })
    });

    let fn_name = &item_fn.sig.ident;
    let struct_name = match name_override {
        Some(n) => format_ident!("{}", n),
        None => format_ident!("{}Op", to_pascal_case(&fn_name.to_string())),
    };
    let vis = item_fn.vis.clone();
    item_fn.vis = Visibility::Inherited;

    let availability_impl = is_available.map(|path| {
        quote! {
            fn register_availability_check(
                commands: &mut ::bevy::ecs::system::Commands,
            ) -> ::core::option::Option<::bevy::ecs::system::SystemId<(), bool>> {
                ::core::option::Option::Some(commands.register_system(#path))
            }
        }
    });

    let cancel_impl = cancel.map(|path| {
        quote! {
            fn register_cancel(
                commands: &mut ::bevy::ecs::system::Commands,
            ) -> ::core::option::Option<::bevy::ecs::system::SystemId<()>> {
                ::core::option::Option::Some(commands.register_system(#path))
            }
        }
    });

    let expanded = quote! {
        #[derive(::core::default::Default, ::bevy_enhanced_input::prelude::InputAction)]
        #[action_output(bool)]
        #vis struct #struct_name;

        impl ::jackdaw_api::prelude::Operator for #struct_name {
            const ID: &'static str = #id;
            const LABEL: &'static str = #label;
            const DESCRIPTION: &'static str = #description;
            const MODAL: bool = #modal;
            const ALLOWS_UNDO: bool = #allows_undo;

            fn register_execute(
                commands: &mut ::bevy::ecs::system::Commands,
            ) -> ::jackdaw_api::prelude::OperatorSystemId {
                commands.register_system(#fn_name)
            }

            #availability_impl

            #cancel_impl
        }

        #item_fn
    };

    expanded.into()
}

fn as_lit_str(expr: &Expr) -> Option<LitStr> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Str(s), ..
    }) = expr
    {
        Some(s.clone())
    } else {
        None
    }
}

fn as_str_expr(expr: &Expr) -> Option<Expr> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(_), ..
        }) => Some(expr.clone()),

        Expr::Path(_) => Some(expr.clone()),

        _ => None,
    }
}

fn as_lit_bool(expr: &Expr) -> Option<LitBool> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Bool(b), ..
    }) = expr
    {
        Some(b.clone())
    } else {
        None
    }
}

fn as_path(expr: &Expr) -> Option<Path> {
    if let Expr::Path(ExprPath { path, .. }) = expr {
        Some(path.clone())
    } else {
        None
    }
}

fn to_pascal_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    for part in snake.split('_') {
        let mut chars = part.chars();
        if let Some(c) = chars.next() {
            out.extend(c.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}
