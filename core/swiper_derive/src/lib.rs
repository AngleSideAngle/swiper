extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{
    parse_macro_input, punctuated::Punctuated, spanned::Spanned, token::Token, FnArg, Ident, ItemFn,
};

// two macros
// #[preemptible] (for functions) replaces all function args T with RevocableCell<T>, doesn't touch receivers
// #[impl_preemptible] (for impl blocks) generates an impl for Receiver<T> will all methods for receiver T

#[proc_macro_attribute]
pub fn preemptible(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemFn);
    let names: Vec<syn::Ident> =
        parse_macro_input!(attr with Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated)
            .into_iter()
            .collect();

    // ensure function is async
    if input.sig.asyncness.is_none() {
        let span = input.sig.fn_token.span;
        return TokenStream::from(quote_spanned! {
            span => compile_error!("function must be async to safetly handle preemption");
        });
    }

    // replace all types of function args that match proc macro arg identity with RevocableCell<>
    for arg in &mut input.sig.inputs {
        if let FnArg::Typed(pattern) = arg {
            if let syn::Pat::Ident(identity) = &*pattern.pat {
                // default behavior is wrap all arguments
                if names.is_empty() || names.contains(&identity.ident) {
                    let old_type = pattern.ty.clone();
                    pattern.ty = Box::new(syn::parse_quote!( RevocableCell<#old_type> ));
                }
            }
        }
        // ignore receivers to avoid breaking method calling
    }

    let fn_name = &input.sig.ident;
    let fn_block = &input.block;
    let fn_sig = &input.sig;
    let fn_attrs = &input.attrs;
    let fn_vis = &input.vis;

    let mapped_args = input.sig.inputs.into_iter().map(|arg| arg).collect();

    let result = quote! {
        #(#fn_attrs)*
        #fn_vis #fn_sig {
            // for each requirement name, arg.data.get()
            // each non requirement name is just arg
            // internal async function is
            let inner = #fn_block();
            // create requirements array with all RevocableCells
            // create empty current_flags array of Option::None of same length
            PreemptibleFuture { inner, requirements, current_flags }
        }
    };

    result.into()
}
