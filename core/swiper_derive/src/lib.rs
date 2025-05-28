extern crate proc_macro;

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, spanned::Spanned, FnArg, Ident, ItemFn,
    Pat,
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
        let span = input.sig.fn_token.span;
        return TokenStream::from(quote_spanned! {
            span => compile_error!("function must be async to safetly handle preemption");
        })
        .into();
    }

    // all mutex_args types wrapped with RevocableCell
    let mut outer_params: Vec<FnArg> = Vec::with_capacity(input.sig.inputs.len());

    // original params for inner fn definition
    let mut inner_params: Vec<FnArg> = Vec::with_capacity(input.sig.inputs.len());

    // args to be fed to origin params, all mutex_args mapped x -> unsafe { *x.data.get() }
    let mut inner_args: Vec<proc_macro2::TokenStream> = Vec::with_capacity(input.sig.inputs.len());

    // maps all RevocableCell inputs (a, d, e) -> [&a, &d, &e]
    let mut requirements_arr = Vec::new();

    for arg in &input.sig.inputs {
        match &arg {
            FnArg::Typed(pat) => {
                if let Pat::Ident(ident) = &*pat.pat {
                    if macro_args.is_empty() || macro_args.contains(&ident.ident) {
                        outer_params.push({
                            let old_ty = &*pat.ty;
                            let mut new_pat = pat.clone();
                            new_pat.ty = parse_quote!( RevocableCell<#old_ty> );
                            FnArg::Typed(new_pat)
                        });
                        inner_params.push(FnArg::Typed(pat.clone()));
                        inner_args.push(quote! { unsafe { (*#pat).data.get() } });
                        requirements_arr.push(quote! { &#ident });
                    }
                } else {
                    return TokenStream::from(quote_spanned! {
                        pat.span() => compile_error!("this macro does not yet support destructuring function arguments");
                    }).into();
                }
            }
            FnArg::Receiver(recv) => {
                outer_params.push(arg.clone());
                inner_params.push(arg.clone());
                inner_args.push(recv.self_token.to_token_stream());
            }
        }
    }

    let mut outer_sig = input.sig.clone();
    outer_sig.asyncness = None;
    outer_sig.inputs.clear();
    outer_sig.inputs.extend(outer_params);

    let mut inner_sig = input.sig.clone();
    inner_sig.ident = parse_quote!("inner");
    inner_sig.inputs.clear();
    inner_sig.inputs.extend(inner_params);

    let fn_block = &input.block;
    let fn_attrs = &input.attrs;
    let fn_vis = &input.vis;

    let result = quote! {
        #(#fn_attrs)*
        #fn_vis #outer_sig {
            fn #inner_sig {
                #fn_block
            }

            PreemptibleFuture {
                inner: inner(#(#inner_args),*),
                requirements: [#(#requirements_arr),*],
                current_flags: Default::default()
            }
        }
    };

    result.into()
    // result.into()
}

/// original function + modified inputs -> rust code
fn generate_wrapped_function(
    input: ItemFn,
    outer_params: Vec<FnArg>,
    inner_params: Vec<FnArg>,
    inner_args: Vec<TokenStream>,
    requirements_arr: Vec<TokenStream>,
) -> TokenStream {
    let mut outer_sig = input.sig.clone();
    outer_sig.asyncness = None;
    outer_sig.inputs.clear();
    outer_sig.inputs.extend(outer_params);
    outer_sig.output = quote! { PreemptibleFuture<'_, #outer_sig.output> };

    let mut inner_sig = input.sig.clone();
    inner_sig.ident = parse_quote!("inner");
    inner_sig.inputs.clear();
    inner_sig.inputs.extend(inner_params);

    let fn_block = &input.block;
    let fn_attrs = &input.attrs;
    let fn_vis = &input.vis;

    quote! {
        #(#fn_attrs)*
        #fn_vis #outer_sig {
            #inner_sig {
                #fn_block
            }

            PreemptibleFuture {
                inner: inner(#(#inner_args),*),
                requirements: [#(#requirements_arr),*],
                current_flags: core::Default::default()
            }
        }
    }
}
