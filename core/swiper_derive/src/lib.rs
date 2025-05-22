extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn time(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // TODO figure out what i'm doing
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let fn_block = &input.block;
    let fn_sig = &input.sig;
    let fn_attrs = &input.attrs;
    let fn_vis = &input.vis;

    let result = quote! {
        #(#fn_attrs)*
        #fn_vis #fn_sig {
            let __result = (|| #fn_block)();
            __result
        }
    };

    result.into()
}
