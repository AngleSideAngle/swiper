extern crate proc_macro;

use core::fmt;

use quote::{ToTokens, format_ident};
use syn::{
    Error, Expr, FnArg, Ident, ItemFn, Pat, PatType, ReturnType, parse_macro_input, parse_quote,
    punctuated::Punctuated,
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
            "function must be async to safetly be preempted",
        )
        .into_compile_error()
        .into();
    }

    let ir = match single_fn_to_ir(&input, &macro_args) {
        Ok(ir) => ir,
        Err(e) => return e.into_compile_error().into(),
    };

    generate_wrapped_function(&input, ir)
        .into_token_stream()
        .into()
}

struct IntermediateRepr {
    outer_params: Vec<FnArg>,
    inner_params: Vec<FnArg>,
    inner_args: Vec<Expr>,
    requirements_arr: Vec<Expr>,
}

impl fmt::Debug for IntermediateRepr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn tokens_to_strings<T: ToTokens>(items: &[T]) -> Vec<String> {
            items
                .iter()
                .map(|item| item.to_token_stream().to_string())
                .collect()
        }

        f.debug_struct("IntermediateRepr")
            .field("outer_params", &tokens_to_strings(&self.outer_params))
            .field("inner_params", &tokens_to_strings(&self.inner_params))
            .field("inner_args", &tokens_to_strings(&self.inner_args))
            .field(
                "requirements_arr",
                &tokens_to_strings(&self.requirements_arr),
            )
            .finish()
    }
}

impl PartialEq for IntermediateRepr {
    fn eq(&self, other: &Self) -> bool {
        fn token_eq<T: ToTokens>(a: &[T], b: &[T]) -> bool {
            if a.len() != b.len() {
                return false;
            }
            a.iter()
                .zip(b)
                .all(|(x, y)| x.to_token_stream().to_string() == y.to_token_stream().to_string())
        }

        token_eq(&self.outer_params, &other.outer_params)
            && token_eq(&self.inner_params, &other.inner_params)
            && token_eq(&self.inner_args, &other.inner_args)
            && token_eq(&self.requirements_arr, &other.requirements_arr)
    }
}

fn method_to_ir() -> syn::Result<IntermediateRepr> {
    todo!()
}

// parses input args to create intermediate representation
fn single_fn_to_ir(input: &ItemFn, wrapped_names: &[Ident]) -> syn::Result<IntermediateRepr> {
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
            FnArg::Typed(PatType { attrs, pat, ty, .. }) => {
                if let Pat::Ident(ident) = &**pat {
                    if wrapped_names.is_empty() || wrapped_names.contains(&ident.ident) {
                        outer_params.push(parse_quote! {
                            #(#attrs)*
                            #pat: &swiper_stealing::RevocableCell<#ty>
                        });
                        inner_params.push(parse_quote! { #pat: #ty });
                        inner_args.push(parse_quote! { unsafe { *#pat.data.get() } });
                        requirements_arr.push(parse_quote! { &#ident });
                    } else {
                        outer_params.push(parse_quote! {
                            #(#attrs)*
                            #pat: #ty
                        });
                        inner_params.push(parse_quote! { #pat: #ty });
                        inner_args.push(parse_quote! { #pat });
                    }
                } else {
                    return Err(Error::new_spanned(
                        pat,
                        "this macro does not yet support destructuring function arguments",
                    ));
                }
            }
            FnArg::Receiver(recv) => {
                outer_params.push(arg.clone());
                inner_params.push(arg.clone());
                inner_args.push(parse_quote!(#recv.self_token));
            }
        }
    }

    Ok(IntermediateRepr {
        outer_params,
        inner_params,
        inner_args,
        requirements_arr,
    })
}

/// original function + modified inputs -> rust code
fn generate_wrapped_function(
    input: &ItemFn,
    IntermediateRepr {
        outer_params,
        inner_params,
        inner_args,
        requirements_arr,
    }: IntermediateRepr,
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
        Box::new(
            parse_quote! { core::result::Result<#prev_output, swiper_stealing::PreemptionError> },
        ),
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

            swiper_stealing::PreemptibleFuture::new(
                inner(#(#inner_args),*),
                [#(#requirements_arr),*],
            ).await
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
            IntermediateRepr {
                outer_params: vec![parse_quote! { x: i32 }],
                inner_params: vec![parse_quote! { x: i32 }],
                inner_args: vec![parse_quote! { x }],
                requirements_arr: vec![],
            },
        )
        .into_token_stream()
        .to_string();

        let expected = quote::quote! {
            async fn eg(x: i32) -> core::result::Result<i32, swiper_stealing::PreemptionError> {
                async fn inner(x: i32) -> i32 {
                    x
                }

                swiper_stealing::PreemptibleFuture::new(
                    inner(x),
                    [],
                ).await
            }
        }
        .to_string();

        assert_eq!(out, expected);
    }

    #[test]
    fn wrapped_fn_success_2() {
        let out = generate_wrapped_function(
            &parse_quote! { async fn eg(x: i32, y: i32) -> i32 { x + y } },
            IntermediateRepr {
                outer_params: vec![
                    parse_quote! { x: &swiper_stealing::RevocableCell<i32> },
                    parse_quote! { y: i32 },
                ],
                inner_params: vec![parse_quote! { x: i32 }, parse_quote! { y: i32 }],
                inner_args: vec![
                    parse_quote! { unsafe { *x.data.get() } },
                    parse_quote! { y },
                ],
                requirements_arr: vec![parse_quote! { &x }],
            },
        )
        .into_token_stream()
        .to_string();

        let expected = quote::quote! {
            async fn eg(x: &swiper_stealing::RevocableCell<i32>, y: i32) -> core::result::Result<i32, swiper_stealing::PreemptionError> {
                async fn inner(x: i32, y: i32) -> i32 {
                    x + y
                }

                swiper_stealing::PreemptibleFuture::new(
                    inner( unsafe { *x.data.get() }, y),
                    [&x],
                ).await
            }
        }
        .to_string();

        assert_eq!(out, expected);
    }

    #[test]
    fn fn_to_ir() {
        let out = single_fn_to_ir(
            &parse_quote! { async fn eg(a: i32, b: i32) { a + b } },
            &[format_ident!("a")],
        )
        .expect("failed to parse IR");

        let expected = IntermediateRepr {
            outer_params: vec![
                parse_quote! { a: &swiper_stealing::RevocableCell<i32>},
                parse_quote! { b: i32 },
            ],
            inner_params: vec![parse_quote! { a: i32 }, parse_quote! { b: i32 }],
            inner_args: vec![
                parse_quote! { unsafe { *a.data.get() } },
                parse_quote! { b },
            ],
            requirements_arr: vec![parse_quote! {&a}],
        };

        assert_eq!(out, expected);
    }
}
