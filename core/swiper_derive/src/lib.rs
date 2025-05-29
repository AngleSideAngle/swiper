extern crate proc_macro;

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, spanned::Spanned, Expr, FnArg, Ident,
    ItemFn, Pat, ReturnType, Type,
};

// two macros
// #[preemptible] (for functions) replaces all function args T with RevocableCell<T>, doesn't touch receivers
// #[impl_preemptible] (for impl blocks) generates an impl for Receiver<T> will all methods for receiver T

#[proc_macro_attribute]
pub fn preemptible(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let macro_args: Vec<Ident> =
        parse_macro_input!(attr with Punctuated::<Ident, syn::Token![,]>::parse_terminated)
            .into_iter()
            .collect();

    // ensure function is async
    if input.sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            input.sig.fn_token,
            "function must be async to safetly handle preemption",
        )
        .into_compile_error()
        .into();
    }

    // all mutex_args types wrapped with RevocableCell
    let mut outer_params: Vec<FnArg> = Vec::with_capacity(input.sig.inputs.len());

    // original params for inner fn definition
    let mut inner_params: Vec<FnArg> = Vec::with_capacity(input.sig.inputs.len());

    // args to be fed to origin params, all mutex_args mapped x -> unsafe { *x.data.get() }
    let mut inner_args: Vec<Expr> = Vec::with_capacity(input.sig.inputs.len());

    // maps all RevocableCell inputs (a, d, e) -> [&a, &d, &e]
    let mut requirements_arr: Vec<Expr> = Vec::new();

    for arg in &input.sig.inputs {
        match &arg {
            FnArg::Typed(pat) => {
                if let Pat::Ident(ident) = &*pat.pat {
                    if macro_args.is_empty() || macro_args.contains(&ident.ident) {
                        outer_params.push({
                            let old_ty = &*pat.ty;
                            let mut new_pat = pat.clone();
                            new_pat.ty = parse_quote!( &swiper_stealing::RevocableCell<#old_ty> );
                            FnArg::Typed(new_pat)
                        });
                        inner_params.push(FnArg::Typed(pat.clone()));
                        inner_args.push(parse_quote! { unsafe { (*#pat).data.get() } });
                        requirements_arr.push(parse_quote! { &#ident });
                    }
                } else {
                    return syn::Error::new_spanned(
                        pat,
                        "this macro does not yet support destructuring function arguments",
                    )
                    .to_compile_error()
                    .into();
                }
            }
            FnArg::Receiver(recv) => {
                outer_params.push(arg.clone());
                inner_params.push(arg.clone());
                inner_args.push(parse_quote!(#recv.self_token));
            }
        }
    }

    generate_wrapped_function(
        &input,
        outer_params,
        inner_params,
        &inner_args,
        &requirements_arr,
    )
    .into_token_stream()
    .into()
}

/// original function + modified inputs -> rust code
fn generate_wrapped_function(
    input: &ItemFn,
    outer_params: Vec<FnArg>,
    inner_params: Vec<FnArg>,
    inner_args: &Vec<Expr>,
    requirements_arr: &Vec<Expr>,
) -> ItemFn {
    // both inner and outer signatures are async because i don't know the type of the anonymous inner fn and don't want to parameterize the outer fn on its type
    let mut outer_sig = input.sig.clone();
    outer_sig.inputs.clear();
    outer_sig.inputs.extend(outer_params);

    let prev_output = match outer_sig.output {
        ReturnType::Default => parse_quote! { () },
        ReturnType::Type(_, out) => *out,
    };
    outer_sig.output = ReturnType::Type(
        syn::token::RArrow::default(),
        Box::new(parse_quote! { core::Result<#prev_output, swiper_stealing::PreemptionError> }),
    );

    let mut inner_sig = input.sig.clone();
    inner_sig.ident = format_ident!("inner");
    inner_sig.inputs.clear();
    inner_sig.inputs.extend(inner_params);

    let fn_block = &input.block;
    let fn_attrs = &input.attrs;
    let fn_vis = &input.vis;

    parse_quote! {
        #(#fn_attrs)*
        #fn_vis #outer_sig {
            #inner_sig #fn_block


            swiper_stealing::PreemptibleFuture {
                inner: inner(#(#inner_args),*),
                requirements: [#(#requirements_arr),*],
                current_flags: core::Default::default()
            }.await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_fn_success() {
        let out = generate_wrapped_function(
            &parse_quote! { async fn eg(x: i32) -> i32 { x } },
            vec![parse_quote! { x: i32 }],
            vec![parse_quote! { x: i32 }],
            &vec![parse_quote! { x }],
            &vec![],
        )
        .into_token_stream()
        .to_string();

        let expected = quote! {
            async fn eg(x: i32) -> core::Result<i32, swiper_stealing::PreemptionError> {
                async fn inner(x: i32) -> i32 {
                    x
                }

                swiper_stealing::PreemptibleFuture {
                    inner: inner(x),
                    requirements: [],
                    current_flags: core::Default::default()
                }.await
            }
        }
        .to_string();

        assert_eq!(out, expected);
    }

    #[test]
    fn wrapped_fn_success_2() {
        let out = generate_wrapped_function(
            &parse_quote! { async fn eg(x: i32, y: i32) -> i32 { x + y } },
            vec![
                parse_quote! { x: &swiper_stealing::RevocableCell<i32> },
                parse_quote! { y: i32 },
            ],
            vec![parse_quote! { x: i32 }, parse_quote! { y: i32 }],
            &vec![
                parse_quote! { unsafe { *x.data.get() } },
                parse_quote! { y },
            ],
            &vec![parse_quote! { &x }],
        )
        .into_token_stream()
        .to_string();

        let expected = quote! {
            async fn eg(x: &swiper_stealing::RevocableCell<i32>, y: i32) -> core::Result<i32, swiper_stealing::PreemptionError> {
                async fn inner(x: i32, y: i32) -> i32 {
                    x + y
                }

                swiper_stealing::PreemptibleFuture {
                    inner: inner( unsafe { *x.data.get() }, y),
                    requirements: [&x],
                    current_flags: core::Default::default()
                }.await
            }
        }
        .to_string();

        assert_eq!(out, expected);
    }
}
