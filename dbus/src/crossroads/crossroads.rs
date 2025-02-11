use std::collections::{BTreeMap, HashMap};
use std::any::{TypeId, Any};
use std::ffi::{CString, CStr};
use std::fmt;
use crate::strings::{Path as PathName, Interface as IfaceName, Member as MemberName, Signature};
use crate::{Message, MessageType};
use super::info::{IfaceInfo, MethodInfo, PropInfo, IfaceInfoBuilder};
use super::handlers::{Handlers, Par, ParInfo, Mut, MutCtx, MutMethods};
use super::stdimpl::{DBusProperties, DBusIntrospectable};

// The key is an IfaceName, but if we have that we bump into https://github.com/rust-lang/rust/issues/59732
// so we use CString as a workaround.
//#[derive(Default, Debug)]
//struct IfaceReg<H: Handlers>(BTreeMap<CString, (TypeId, IfaceInfo<'static, H>)>);

#[derive(Default)]
pub struct PathData<H: Handlers>(HashMap<TypeId, H::Iface>);

impl PathData<Par> {
    pub fn insert_par<I: Any + 'static + Send + Sync>(&mut self, i: I) {
        let id = TypeId::of::<I>();
        let t = Box::new(i);
        self.0.insert(id, t);
    }
}

impl PathData<Mut> {
    pub fn insert_mut<I: Any + 'static>(&mut self, i: I) {
        let id = TypeId::of::<I>();
        let t = Box::new(i);
        self.0.insert(id, t);
    }
}

impl<H: Handlers> PathData<H> {
    pub (super) fn contains_key(&self, x: TypeId) -> bool { self.0.contains_key(&x) }
}

/*
impl<H: Handlers> PathData<H> {
    pub fn get_mut<I: Any + 'static>(&mut self) -> Option<&mut I> {
        let id = TypeId::of::<I>();
        self.0.get_mut(&id).and_then(|x| (&mut x as &mut dyn Any).downcast_mut())
    }
}
*/
impl<H: Handlers> fmt::Debug for PathData<H> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "...") }
}

impl<H: Handlers> PathData<H> {
    fn new() -> Self { PathData(Default::default()) }
}

//#[derive(Debug)]
//struct IfacePaths<H: Handlers>(BTreeMap<CString, PathData<H>>);

//impl<H: Handlers> Default for IfacePaths<H> {
//    fn default() -> Self { IfacePaths(BTreeMap::new()) }
//}

struct MsgHeaders<'a> {
    m: MemberName<'a>,
    i: IfaceName<'a>,
    p: PathName<'a>,
}

fn msg_headers(msg: &Message) -> Option<MsgHeaders> {
    if msg.msg_type() != MessageType::MethodCall { return None };
    let p = msg.path()?;
    let i = msg.interface()?;
    let m = msg.member()?;
    Some(MsgHeaders { m, i, p })
}

#[derive(Debug)]
pub (super) struct MLookup<'a, H: Handlers> {
    pub (super) cr: &'a Crossroads<H>,
    pub (super) data: &'a PathData<H>,
    pub (super) iface: &'a H::Iface,
    pub (super) iinfo: &'a IfaceInfo<'static, H>,
//    pub (super) minfo: Option<&'a MethodInfo<'static, H>>,
//    pub (super) pinfo: Option<&'a PropInfo<'static, H>>,
}

#[derive(Debug)]
pub struct Crossroads<H: Handlers> {
    pub (super) reg: BTreeMap<CString, (TypeId, IfaceInfo<'static, H>)>,
    pub (super) paths: BTreeMap<CString, PathData<H>>,
}

impl<H: Handlers> Crossroads<H> {

    pub fn register_custom<I: 'static>(&mut self, info: IfaceInfo<'static, H>) -> Option<IfaceInfo<'static, H>> {
        self.reg.insert(info.name.clone().into_cstring(), (TypeId::of::<I>(), info)).map(|x| x.1)
    }
    pub fn insert<N: Into<PathName<'static>>>(&mut self, name: N, data: PathData<H>) {
        self.paths.insert(name.into().into_cstring(), data);
    }
    pub fn get_data<N: Into<PathName<'static>>>(&self, name: N) -> Option<&PathData<H>> {
        self.paths.get(name.into().as_cstr())
    }

    pub fn register<'a, I: 'static, N: Into<IfaceName<'static>>>(&'a mut self, name: N) -> IfaceInfoBuilder<'a, I, H> {
        IfaceInfoBuilder::new(Some(self), name.into())
    }

    fn reg_lookup(&self, headers: &MsgHeaders) -> Option<(MLookup<H>, &MethodInfo<'static, H>)> {
        let (typeid, iinfo) = self.reg.get(headers.i.as_cstr())?;
        let minfo = iinfo.methods.iter().find(|x| x.name() == &headers.m)?;
        let data = self.paths.get(headers.p.as_cstr())?;
        let iface = data.0.get(typeid)?;
        Some((MLookup { cr: self, data, iface, iinfo }, minfo))
    }

    pub (super) fn reg_prop_lookup<'a>(&'a self, data: &'a PathData<H>, iname: &CStr, propname: &CStr) ->
    Option<(MLookup<'a, H>, &PropInfo<'static, H>)> {
        let (typeid, iinfo) = self.reg.get(iname)?;
        let pinfo = iinfo.props.iter().find(|x| x.name.as_cstr() == propname)?;
        let iface = data.0.get(typeid)?;
        Some((MLookup { cr: self, data, iface, iinfo}, pinfo))
    }

    pub (super) fn prop_lookup_mut<'a>(&'a mut self, path: &CStr, iname: &CStr, propname: &CStr) ->
    Option<(&'a mut PropInfo<'static, H>, &'a mut PathData<H>)> {
        let (typeid, iinfo) = self.reg.get_mut(iname)?;
        let propinfo = iinfo.props.iter_mut().find(|x| x.name.as_cstr() == propname)?;
        let path = self.paths.get_mut(path)?;
        Some((propinfo, path))
    }
}

impl Crossroads<Par> {
    pub fn dispatch_par(&self, msg: &Message) -> Option<Vec<Message>> {
        let headers = msg_headers(msg)?;
        let (lookup, minfo) = self.reg_lookup(&headers)?;
        let handler = minfo.handler();
        let iface = &**lookup.iface;
        let mut info = ParInfo::new(msg, lookup);
        let r = (handler)(iface, &mut info);
        Some(r.into_iter().collect())
    }

    pub fn new_par() -> Self { 
        let mut cr = Crossroads {
            reg: BTreeMap::new(),
            paths: BTreeMap::new(),
        };
        DBusProperties::register_par(&mut cr);
        DBusIntrospectable::register(&mut cr);
        cr
    }
}

impl Crossroads<Mut> {
    fn dispatch_ref(&self, msg: &Message, headers: MsgHeaders) -> Option<Vec<Message>> {
        let (_, iinfo) = self.reg.get(headers.i.as_cstr())?;
        let minfo = iinfo.methods.iter().find(|x| x.name() == &headers.m)?;
        let ctx = MutCtx::new(msg);
        let r = match minfo.handler().0 {
            MutMethods::MutIface(_) => unreachable!(),
            MutMethods::AllRef(ref f) => {
                let data = self.paths.get(headers.p.as_cstr())?;
                f(self, data, &ctx)
            },
        };
        Some(r.into_iter().collect())
    }

    pub fn dispatch_mut(&mut self, msg: &Message) -> Option<Vec<Message>> {
        let headers = msg_headers(msg)?;
        let mut try_ref = false;
        let r = {
            let (typeid, iinfo) = self.reg.get_mut(headers.i.as_cstr())?;
            let minfo = iinfo.methods.iter_mut().find(|x| x.name() == &headers.m)?;
            let ctx = MutCtx::new(msg);
            match minfo.handler_mut().0 {
                MutMethods::MutIface(ref mut f) => {
                    let data = self.paths.get_mut(headers.p.as_cstr())?;
                    let iface = data.0.get_mut(typeid)?;
                    let iface = &mut **iface;
                    f(iface, &ctx)
                },
                MutMethods::AllRef(_) => { try_ref = true; None } 
            }
        };
        if try_ref { self.dispatch_ref(msg, headers) }
        else { Some(r.into_iter().collect()) }
    }

    pub fn new_mut() -> Self { 
        let mut cr = Crossroads {
            reg: BTreeMap::new(),
            paths: BTreeMap::new(),
        };
        DBusIntrospectable::register(&mut cr);
        // DBusProperties::register(&mut cr);
        cr
    }
}


#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_send_sync() {
        fn is_send<T: Send>(_: &T) {}
        fn is_sync<T: Sync>(_: &T) {}
        let c = Crossroads::new_par();
        dbg!(&c);
        is_send(&c);
        is_sync(&c);
   }

    #[test]
    fn cr_mut() {
        let mut cr = Crossroads::new_mut();

        struct Score(u16);

        let mut call_times = 0u32;
        cr.register::<Score,_>("com.example.dbusrs.crossroads.score")
            .annotate("com.example.dbusrs.whatever", "Funny annotation")
            .method("UpdateScore", ("change",), ("new_score", "call_times"), move |score: &mut Score, _: &MutCtx, (change,): (u16,)| {
                score.0 += change;
                call_times += 1;
                Ok((score.0, call_times))
            }).deprecated();

        let mut pdata = PathData::new();
        pdata.insert_mut(Score(7u16));
        pdata.insert_mut(DBusIntrospectable);
        cr.insert("/", pdata);

        let msg = Message::new_method_call("com.example.dbusrs.crossroads.score", "/", "com.example.dbusrs.crossroads.score", "UpdateScore").unwrap();
        let mut msg = msg.append1(5u16);
        crate::message::message_set_serial(&mut msg, 57);
        let mut r = cr.dispatch_mut(&msg).unwrap();
        assert_eq!(r.len(), 1);
        r[0].as_result().unwrap();
        let (new_score, call_times): (u16, u32) = r[0].read2().unwrap();
        assert_eq!(new_score, 12);
        assert_eq!(call_times, 1);

        let mut msg = Message::new_method_call("com.example.dbusrs.crossroads.score", "/", "org.freedesktop.DBus.Introspectable", "Introspect").unwrap();
        crate::message::message_set_serial(&mut msg, 57);
        let mut r = cr.dispatch_mut(&msg).unwrap();
        assert_eq!(r.len(), 1);
        r[0].as_result().unwrap();
        let xml_data: &str = r[0].read1().unwrap();
        println!("{}", xml_data);
        // assert_eq!(xml_data, "mooh");
    }


    #[test]
    fn cr_par() {
        let mut cr = Crossroads::new_par();

        struct Score(u16);

        cr.register::<Score,_>("com.example.dbusrs.crossroads.score")
            .method("Hello", ("sender",), ("reply",), |score: &Score, _: &ParInfo, (sender,): (String,)| {
                assert_eq!(score.0, 7u16);
                Ok((format!("Hello {}, my score is {}!", sender, score.0),))
            })
            .prop_ro("Score", |score, _| {
                assert_eq!(score.0, 7u16);
                Ok(score.0)
            }).emits_changed(super::super::info::EmitsChangedSignal::False)
            .signal::<(u16,),_>("ScoreChanged", ("NewScore",));

        let mut pdata = PathData::new();
        pdata.insert_par(Score(7u16));
        pdata.insert_par(DBusProperties);
        pdata.insert_par(DBusIntrospectable);
        cr.insert("/", pdata);

        let msg = Message::new_method_call("com.example.dbusrs.crossroads.score", "/", "com.example.dbusrs.crossroads.score", "Hello").unwrap();
        let mut msg = msg.append1("example");
        crate::message::message_set_serial(&mut msg, 57);
        let mut r = cr.dispatch_par(&msg).unwrap();
        assert_eq!(r.len(), 1);
        r[0].as_result().unwrap();
        let rr: String = r[0].read1().unwrap();
        assert_eq!(&rr, "Hello example, my score is 7!");

        let msg = Message::new_method_call("com.example.dbusrs.crossroads.score", "/", "org.freedesktop.DBus.Properties", "Get").unwrap();
        let mut msg = msg.append2("com.example.dbusrs.crossroads.score", "Score");
        crate::message::message_set_serial(&mut msg, 57);
        let mut r = cr.dispatch_par(&msg).unwrap();
        assert_eq!(r.len(), 1);
        r[0].as_result().unwrap();
        let z: u16 = r[0].read1().unwrap();
        assert_eq!(z, 7u16);

        let mut msg = Message::new_method_call("com.example.dbusrs.crossroads.score", "/", "org.freedesktop.DBus.Introspectable", "Introspect").unwrap();
        crate::message::message_set_serial(&mut msg, 57);
        let mut r = cr.dispatch_par(&msg).unwrap();
        assert_eq!(r.len(), 1);
        r[0].as_result().unwrap();
        let xml_data: &str = r[0].read1().unwrap();
        println!("{}", xml_data);
    }
}
