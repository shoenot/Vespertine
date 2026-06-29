use proc_macro::TokenStream;
use quote::{
    format_ident,
    quote,
};
use syn::{
    ItemFn,
    ReturnType,
    parse_macro_input,
};

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut function = parse_macro_input!(item as ItemFn);

    if function.sig.asyncness.is_some() {
        return syn::Error::new_spanned(function.sig.asyncness, "vespertine_app::main does not support async functions")
            .to_compile_error()
            .into();
    }

    if !function.sig.generics.params.is_empty() {
        return syn::Error::new_spanned(&function.sig.generics, "vespertine_app::main does not support generic functions")
            .to_compile_error()
            .into();
    }

    let original_name = function.sig.ident.clone();
    let inner_name = format_ident!("__vespertine_app_{}", original_name);

    function.sig.ident = inner_name.clone();

    let call = if function.sig.inputs.is_empty() {
        quote! { #inner_name() }
    } else if function.sig.inputs.len() == 1 {
        quote! { #inner_name(pkg) }
    } else {
        return syn::Error::new_spanned(
            &function.sig.inputs,
            "vespertine_app::main expects zero arguments or one ProcessInitPackage reference",
        )
        .to_compile_error()
        .into();
    };

    let body = match &function.sig.output {
        ReturnType::Default => {
            quote! { #call; }
        }
        ReturnType::Type(..) => {
            quote! {
                if let Err(error) = #call {
                    let out = vstd::typed::TypedWriter::out();
                    let _ = out.error(&*alloc::format!("{:?}", error));
                    let _ = out.stream_end();
                }
            }
        }
    };

    quote! {
        #function

        #[unsafe(no_mangle)]
        pub extern "sysv64" fn main(pkg_ptr: *const vabi::ProcessInitPackage) {
            let pkg = unsafe { &*pkg_ptr };

            #body

            let _ = vrt::syscall::sys_close(vstd::env::sink());
        }
    }
    .into()
}
