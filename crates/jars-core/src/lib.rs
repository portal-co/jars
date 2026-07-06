use std::collections::{BTreeMap, BTreeSet, VecDeque};

use noak::reader::{AttributeContent, Class};
use nom::{
    IResult, Parser,
    bytes::{tag, take_until},
    multi::many0,
};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::Ident;

pub trait ToToTokens<Ctx> {
    fn tokens(&self, ctx: &Ctx) -> impl ToTokens;
}
fn name_ident(a: &str) -> Ident {
    Ident::new(&a.replace(['.', '$', '/'], "_"), Span::call_site())
}
fn parse_type(
    desc: &str,
    generic: Option<Ident>,
) -> IResult<
    &str,
    (
        TokenStream,
        TokenStream,
        Option<(Ident, Ident, TokenStream)>,
    ),
> {
    return (tag("L"), take_until(";"))
        .map(|(_, a): (&str, &str)| {
            let name = name_ident(a);
            let event = format_ident!("{name}Event");
            let methods = format_ident!("{name}Methods");
            match generic.as_ref() {
                Some(generic) => (
                    quote! {
                       #generic
                    },
                    quote! {
                        impl #methods
                    },
                    Some((
                        generic.clone(),
                        name,
                        quote! {
                            #methods
                        },
                    )),
                ),
                None => (
                    quote! {
                        Channel<#event>
                    },
                    quote! {
                        impl #methods
                    },
                    None,
                ),
            }
        })
        .parse(desc);
}
fn parse_bare_type(desc: &str) -> IResult<&str, (TokenStream, TokenStream)> {
    let (a, (x, y, _)) = parse_type(desc, None)?;
    return Ok((a, (x, y)));
}
pub fn classes(classes: &[Class<'_>]) -> impl ToTokens {
    struct ClassInfo {
        fields: Vec<TokenStream>,
        generics: Vec<(Ident, Ident, TokenStream)>,
    }
    let pass1 = classes
        .iter()
        .map(|c| {
            let name = name_ident(
                c.pool()
                    .get(c.pool().get(c.this_class()).unwrap().name)
                    .unwrap()
                    .content
                    .to_str()
                    .unwrap(),
            );
            let event = format_ident!("{name}Event");
            let (fields, generics) = c
                .fields()
                .iter()
                .map(|field| {
                    let field = field.unwrap();
                    let name = name_ident(
                        c.pool()
                            .get(field.name())
                            .unwrap()
                            .content
                            .to_str()
                            .unwrap(),
                    );
                    let desc = c
                        .pool()
                        .get(field.descriptor())
                        .unwrap()
                        .content
                        .to_str()
                        .unwrap();
                    let (_, (ty, _, evt)) = parse_type(desc, Some(name.clone())).unwrap();
                    (
                        quote! {
                            #name : Mutex<#ty>
                        },
                        evt,
                    )
                })
                .collect::<(Vec<_>, Vec<_>)>();
            (
                name,
                ClassInfo {
                    fields,
                    generics: generics.into_iter().flatten().collect(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let bodies = classes.iter().map(|c| {
        let name = name_ident(
            c.pool()
                .get(c.pool().get(c.this_class()).unwrap().name)
                .unwrap()
                .content
                .to_str()
                .unwrap(),
        );
        let some = format_ident!("Some{name}");
        let methods = format_ident!("{name}Methods");
        let bounds = format_ident!("{name}Bounds");
        let default_bounds = format_ident!("Default{name}Bounds");
        let event = format_ident!("{name}Event");
        let ClassInfo { fields, generics } = pass1.get(&name).unwrap();
        let (generic_names, generic_bounds, generic_defaults) = generics
            .iter()
            .flat_map(|(a, b, c)| {
                let event = format_ident!("{b}Event");
                [
                    (
                        a.clone(),
                        quote! {
                            #c
                        },
                        quote! {
                            Channel<#event>
                        },
                    ),
                ]
            })
            .collect::<(Vec<_>, Vec<_>, Vec<_>)>();
        let (method_names, method_types, generic_method_types, bodies, remap_keys, remap_values) = c
            .methods()
            .iter()
            .map(|a| {
                let a = a.unwrap();
                let code = a.attributes().iter().find_map(|a|match a.ok()?.read_content(c.pool()).ok()?{
                    AttributeContent::Code(c) => Some(c),
                    _ => None,
                });
                let (_, (_, params, _, returns)) = (
                    tag("("),
                    many0(|a|parse_bare_type(a).map(|(c,b)|(c,(a,b)))),
                    tag(")"),
                    parse_bare_type.or(tag("V").map(|_| (quote!{()},quote!{()}))),
                )
                    .parse(
                        c.pool()
                            .get(a.descriptor())
                            .unwrap()
                            .content
                            .to_str()
                            .unwrap(),
                    )
                    .unwrap();
                let (param_strs,params) = params.into_iter().collect::<(Vec<_>,Vec<_>)>();
                let (mut params, mut impl_params) = params.into_iter().collect::<(Vec<_>,Vec<_>)>();
                let returns = returns.0;
                let ids = params
                    .iter()
                    .enumerate()
                    .map(|(a, _)| format_ident!("_{a}"))
                    .collect::<Vec<_>>();
                let ids_and_rets = ids
                    .iter()
                    .cloned()
                    .chain([format_ident!("ret")]).collect::<Vec<_>>();
                let return_map = if c.pool()
                            .get(a.descriptor())
                            .unwrap()
                            .content
                            .to_str()
                            .unwrap().ends_with(";"){
                    quote! {
                        ret_val.spawn(spawner)
                    }
                }else{
                    quote! {ret_val}
                };
               
                params.push(quote! {
                    Return<#returns>
                });
                impl_params.push(quote! {
                    Return<#returns>
                });
                (
                    name_ident(c.pool().get(a.name()).unwrap().content.to_str().unwrap()),
                    params,
                    impl_params,
                    quote! {
                        enum TaskEntry<S: Spawner>{
                            Continue(S::Task<Result<TaskEntry<S>,Error>>),
                            Break((#(#returns),))
                        }
                        #(#pcs)*
                        let (#(#ids_and_rets),*) = a;
                        let ret_val = try{
                            match _pc0(spawner.clone(), #(#ids),*).await?{
                                TaskEntry::Break(a) => a,
                                TaskEntry::Continue(mut task) => loop{
                                    match task.await?{
                                        TaskEntry::Break(a) => break a,
                                        TaskEntry::Continue(a) => task = a,
                                    }
                                }
                            }
                        }
                        match ret_val{
                            Ok(ret_val) => {
                                ret.send(Ok(#return_map)).await
                            },
                            Err(e) => {
                                ret.send(Err(e)).await
                            }
                        }
                    },
                    ids_and_rets.clone(),
                    param_strs.iter().map(|a|a.ends_with(";")).chain([false]).zip(ids_and_rets).map(|(a,b)|if a{quote!{#b.spawn(spawner)}}else{quote! {#b}}).collect::<Vec<_>>()
                )
            })
            .collect::<(Vec<_>, Vec<_>,Vec<_>, Vec<_>,Vec<_>,Vec<_>)>();
        quote! {
            pub enum #event{
                #(#method_names((#(#method_types),*))),*
            }
            #[named_generics_bundle]
            pub trait #bounds{
                #(type #generic_names : #generic_bounds);*
            }
            pub type #default_bounds = #bounds![
                #(#generic_names = #generic_defaults),*
            ];
            pub trait #methods{
                fn spawn(self, spawner: impl Spawner) -> Channel<#event>;
                async fn handle(&self, evt: #event, spawner: impl Spawner);
                #(async fn #method_names(&self, a: (#(#generic_method_types),*), spawner: impl Spawner);)*
            }
            pub struct #name<X: #bounds>{
                #(#fields),*
            }
            impl<X: #bounds> #methods for #name<X>{
                fn spawn(self, spawner: impl Spawner) -> Channel<#event>{
                    let chan = Channel::new();
                    let rc = chan.clone();
                    let s = spawner.clone();
                    spawner.spawn(async move{
                        loop{
                            let r = rc.recv().await;
                            s.spawn(self.handle(r,s.clone()).await);
                        }
                    });
                    return rc;
                }
                async fn handle(&self, evt: #event, spawner: impl Spawner){
                    Self::handle(self,evt,spawner).await;
                }
                #(async fn #method_names(&self, a: (#(#generic_method_types),*), spawner: impl Spawner){
                    Self::#method_names(self,a,spawner)
                })*
            }
            impl #methods for Channel<#event>{
                fn spawn(self, spawner: impl Spawner) -> Channel<#event>{
                    self
                }
                async fn handle(&self, evt: #event, spawner: impl Spawner){
                    self.send(evt).await;
                } 
                #(async fn #method_names(&self, a: (#(#generic_method_types),*), spawner: impl Spawner){
                    self.send(#event::#method_names(match a{
                        (#(#remap_keys),*) => (#(#remap_values),*)
                    })).await;
                })*
            }
            impl<X: #bounds> #name<X>{
                pub async fn handle(&self, evt: #event, spawner: impl Spawner){
                    match evt{
                        #(#event::#method_names(a) => self.#method_names(a,spawner)) ,*
                    }
                }
                #(pub async fn #method_names(&self, a: (#(#generic_method_types),*), spawner: impl Spawner){
                    #bodies
                })*
            }
        }
    });
    quote! {
        #(#bodies)*
    }
}
