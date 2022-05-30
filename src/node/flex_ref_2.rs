use std::{borrow::Borrow, cmp::Ordering, fmt, marker::PhantomData, ops::Deref, sync::Arc};

use crate::{
    store::{
        unwrap_safe, Blob, Blob2, BlobOwner, BlobStore2 as BlobStore, NoError, NoStore, OwnedBlob,
    },
    Hex,
};
use std::fmt::Debug;

#[repr(C)]
struct FlexRef<T>(PhantomData<T>, [u8]);

impl<T> Debug for FlexRef<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match tpe(self.header()) {
            Type::None => write!(f, "FlexRef::None"),
            Type::Id => write!(f, "FlexRef::Id"),
            Type::Inline => write!(
                f,
                "FlexRef::Inline({})",
                Hex::new(self.inline_as_ref().unwrap())
            ),
            Type::Arc => write!(f, "FlexRef::Arc"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Type {
    None,
    Inline,
    Id,
    Arc,
}

#[repr(transparent)]
struct TreePrefixRef<S = NoStore>(PhantomData<S>, FlexRef<Vec<u8>>);

impl<S: BlobStore> Debug for TreePrefixRef<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TreePrefix({:?})", &self.1)
    }
}

impl<S: BlobStore> TreePrefixRef<S> {
    fn bytes(&self) -> &[u8] {
        self.1.bytes()
    }

    fn is_empty(&self) -> bool {
        self.first_opt().is_none()
    }

    fn first_opt(&self) -> Option<u8> {
        self.1.first_u8_opt()
    }

    fn first(&self) -> u8 {
        self.1.first_u8()
    }

    fn new(value: &FlexRef<Vec<u8>>) -> &Self {
        // todo: use ref_cast
        unsafe { std::mem::transmute(value) }
    }

    fn is_valid(&self) -> bool {
        self.bytes().len() > 0 && self.1.tpe() != Type::None
    }

    fn load2(&self, store: &S) -> Result<TreePrefix<'_>, S::Error> {
        Ok(if let Some(x) = self.1.inline_as_ref() {
            TreePrefix(Blob2::new(x))
        } else if let Some(x) = self.1.arc_as_clone() {
            TreePrefix(x.into())
        } else if let Some(id) = self.1.id_as_slice() {
            TreePrefix::from_blob(store.read(id)?)
        } else {
            panic!()
        })
    }

    fn load(&self, store: &S) -> Result<OwnedTreePrefix, S::Error> {
        Ok(if let Some(x) = self.1.inline_as_ref() {
            TreePrefix::from_slice(x)
        } else if let Some(x) = self.1.arc_as_clone() {
            TreePrefix::from_arc_vec(x)
        } else if let Some(id) = self.1.id_as_slice() {
            TreePrefix::from_blob(store.read(id)?)
        } else {
            panic!()
        })
    }

    fn read(value: &[u8]) -> Option<&Self> {
        Some(Self::new(FlexRef::read(value)?))
    }

    fn read_one<'a>(value: &'a [u8]) -> Option<(&'a Self, &'a [u8])> {
        let (f, rest): (&'a FlexRef<Vec<u8>>, &'a [u8]) = FlexRef::read_one(value)?;
        Some((Self::new(f), rest))
    }

    fn empty() -> &'static Self {
        Self::new(FlexRef::<Vec<u8>>::empty())
    }

    fn manual_drop(&self) {
        self.1.manual_drop();
    }

    fn manual_clone(&self) {
        self.1.manual_clone();
    }
}

impl AsRef<[u8]> for TreePrefixRef {
    fn as_ref(&self) -> &[u8] {
        todo!()
    }
}

enum OwnedSlice {
    Arc(Arc<Vec<u8>>),
    Inline(Vec<u8>),
    /// < TODO FIX
    Blob(Blob),
}

impl From<&[u8]> for OwnedSlice {
    fn from(v: &[u8]) -> Self {
        Self::Inline(v.to_vec())
    }
}

impl From<Vec<u8>> for OwnedSlice {
    fn from(v: Vec<u8>) -> Self {
        Self::Inline(v)
    }
}

impl From<Arc<Vec<u8>>> for OwnedSlice {
    fn from(v: Arc<Vec<u8>>) -> Self {
        Self::Arc(v)
    }
}

impl OwnedSlice {
    fn empty() -> Self {
        Self::Inline(Vec::new())
    }

    fn from_blob(blob: Blob) -> Self {
        Self::Blob(blob)
    }

    fn from_arc_vec(arc: Arc<Vec<u8>>) -> Self {
        Self::Arc(arc)
    }

    fn from_slice(v: &[u8]) -> Self {
        Self::Inline(v.to_vec())
    }
}

impl AsRef<[u8]> for OwnedSlice {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Arc(x) => x.as_ref().as_ref(),
            Self::Inline(x) => x.as_ref(),
            Self::Blob(blob) => blob.as_ref(),
        }
    }
}

impl Deref for OwnedSlice {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl fmt::Debug for OwnedSlice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Hex::new(self.as_ref()))
    }
}

#[derive(Debug)]
pub struct TreeValueRefWrapper<S = NoStore>(OwnedBlob, PhantomData<S>);

impl<S: BlobStore> AsRef<TreeValueRef<S>> for TreeValueRefWrapper<S> {
    fn as_ref(&self) -> &TreeValueRef<S> {
        TreeValueRef::new(FlexRef::new(self.0.as_ref()))
    }
}

impl AsRef<[u8]> for TreeValueRefWrapper {
    fn as_ref(&self) -> &[u8] {
        let t: &TreeValueRef<NoStore> = self.as_ref();
        unwrap_safe(t.data(&NoStore))
    }
}

impl Deref for TreeValueRefWrapper {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

pub struct TreeValue<'a>(Blob2<'a>);

pub type OwnedTreeValue = TreeValue<'static>;

impl<'a> Debug for TreeValue<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TreeValue")
            .field(&Hex::new(self.as_ref()))
            .finish()
    }
}

impl<'a> From<&[u8]> for TreeValue<'a> {
    fn from(v: &[u8]) -> Self {
        Self(v.into())
    }
}

impl<'a> From<Vec<u8>> for TreeValue<'a> {
    fn from(v: Vec<u8>) -> Self {
        Self(v.into())
    }
}

impl<'a> From<Arc<Vec<u8>>> for TreeValue<'a> {
    fn from(v: Arc<Vec<u8>>) -> Self {
        Self(v.into())
    }
}

impl<'a> TreeValue<'a> {
    fn empty() -> Self {
        Self(Blob2::empty())
    }

    fn from_blob(blob: OwnedBlob) -> Self {
        Self(blob)
    }

    fn from_arc_vec(arc: Arc<Vec<u8>>) -> Self {
        Self(arc.into())
    }

    fn from_slice(v: &[u8]) -> Self {
        Self(v.into())
    }
}

impl<'a> AsRef<[u8]> for TreeValue<'a> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<'a> Deref for TreeValue<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct TreePrefix<'a>(Blob2<'a>);

type OwnedTreePrefix = TreePrefix<'static>;

impl<'a> From<&[u8]> for TreePrefix<'a> {
    fn from(v: &[u8]) -> Self {
        Self(v.into())
    }
}

impl<'a> From<Vec<u8>> for TreePrefix<'a> {
    fn from(v: Vec<u8>) -> Self {
        Self(v.into())
    }
}

impl<'a> From<Arc<Vec<u8>>> for TreePrefix<'a> {
    fn from(v: Arc<Vec<u8>>) -> Self {
        Self(v.into())
    }
}

impl<'a> TreePrefix<'a> {
    fn append<S: BlobStore>(&mut self, that: &TreePrefixRef<S>, store: &S) -> Result<(), S::Error> {
        todo!()
    }

    fn empty() -> Self {
        Self(Blob2::empty())
    }

    fn from_blob(blob: OwnedBlob) -> Self {
        Self(blob)
    }

    fn from_arc_vec(arc: Arc<Vec<u8>>) -> Self {
        Self(arc.into())
    }

    fn from_slice(v: &[u8]) -> Self {
        Self(v.into())
    }
}

impl<'a> AsRef<[u8]> for TreePrefix<'a> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<'a> Deref for TreePrefix<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[repr(transparent)]
pub struct TreeValueRef<S: BlobStore = NoStore>(PhantomData<S>, FlexRef<Vec<u8>>);

impl<S: BlobStore> Debug for TreeValueRef<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TreeValue({:?})", &self.1)
    }
}

impl<S: BlobStore> TreeValueRef<S> {
    fn new(value: &FlexRef<Vec<u8>>) -> &Self {
        // todo: use ref_cast
        unsafe { std::mem::transmute(value) }
    }

    fn bytes(&self) -> &[u8] {
        self.1.bytes()
    }

    fn is_valid(&self) -> bool {
        self.bytes().len() > 0 && self.1.tpe() != Type::None
    }

    fn load2(&self, store: &S) -> Result<TreeValue<'_>, S::Error> {
        Ok(if let Some(x) = self.1.inline_as_ref() {
            TreeValue(Blob2::new(x))
        } else if let Some(x) = self.1.arc_as_clone() {
            TreeValue(Blob2::from(x))
        } else if let Some(id) = self.1.id_as_slice() {
            TreeValue::from_blob(store.read(id)?)
        } else {
            panic!()
        })
    }

    fn load(&self, store: &S) -> Result<OwnedTreeValue, S::Error> {
        Ok(if let Some(x) = self.1.inline_as_ref() {
            TreeValue::from_slice(x)
        } else if let Some(x) = self.1.arc_as_clone() {
            TreeValue::from_arc_vec(x)
        } else if let Some(id) = self.1.id_as_slice() {
            TreeValue::from_blob(store.read(id)?)
        } else {
            panic!()
        })
    }

    fn data(&self, store: &S) -> Result<&[u8], S::Error> {
        Ok(if let Some(x) = self.1.inline_as_ref() {
            x
        } else if let Some(x) = self.1.arc_as_slice() {
            x
        } else if let Some(id) = self.1.id_as_slice() {
            panic!()
        } else {
            panic!()
        })
    }
}

impl TreeValueRef {
    fn to_owned(&self) -> OwnedTreeValue {
        unwrap_safe(self.load(&NoStore))
    }
}

#[repr(transparent)]
struct TreeValueOptRef<S: BlobStore = NoStore>(PhantomData<S>, FlexRef<Vec<u8>>);

impl<S: BlobStore> Debug for TreeValueOptRef<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TreeValueOpt({:?})", &self.1)
    }
}

impl<S: BlobStore> TreeValueOptRef<S> {
    fn new(value: &FlexRef<Vec<u8>>) -> &Self {
        // todo: use ref_cast
        unsafe { std::mem::transmute(value) }
    }

    fn is_valid(&self) -> bool {
        self.bytes().len() > 0
    }

    fn bytes(&self) -> &[u8] {
        self.1.bytes()
    }

    fn value_opt(&self) -> Option<&TreeValueRef<S>> {
        if self.is_none() {
            None
        } else {
            Some(TreeValueRef::new(&self.1))
        }
    }

    fn none() -> &'static Self {
        Self::new(FlexRef::<Vec<u8>>::none())
    }

    fn is_none(&self) -> bool {
        self.1.is_none()
    }

    fn is_some(&self) -> bool {
        self.1.is_some()
    }

    fn load(&self, store: &S) -> Result<Option<OwnedTreeValue>, S::Error> {
        Ok(if let Some(x) = self.1.inline_as_ref() {
            Some(TreeValue::from_slice(x))
        } else if let Some(x) = self.1.arc_as_clone() {
            Some(TreeValue::from_arc_vec(x))
        } else if let Some(id) = self.1.id_as_slice() {
            Some(TreeValue::from_blob(store.read(id)?))
        } else {
            None
        })
    }

    fn read_one<'a>(value: &'a [u8]) -> Option<(&'a Self, &'a [u8])> {
        let (f, rest): (&'a FlexRef<Vec<u8>>, &'a [u8]) = FlexRef::read_one(value)?;
        Some((Self::new(f), rest))
    }

    fn manual_drop(&self) {
        self.1.manual_drop();
    }

    fn manual_clone(&self) {
        self.1.manual_clone();
    }
}

#[repr(transparent)]
struct TreeChildrenRef<S: BlobStore>(PhantomData<S>, FlexRef<NodeSeqBuilder<S>>);

impl<S: BlobStore> Debug for TreeChildrenRef<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.1.tpe() {
            Type::Arc => write!(f, "TreeChildren::Arc({:?})", self.1.arc_as_clone().unwrap()),
            Type::Id => write!(f, "TreeChildren::Id({:?})", self.1.id_as_slice().unwrap()),
            Type::None => write!(f, "TreeChildren::Empty"),
            Type::Inline => write!(f, "TreeChildren::Invalid"),
        }
    }
}

impl<S: BlobStore> TreeChildrenRef<S> {
    fn new(value: &FlexRef<Vec<u8>>) -> &Self {
        // todo: use ref_cast
        unsafe { std::mem::transmute(value) }
    }

    fn bytes(&self) -> &[u8] {
        self.1.bytes()
    }

    fn is_valid(&self) -> bool {
        self.bytes().len() > 0 && self.1.tpe() != Type::Inline
    }

    fn read_one<'a>(value: &'a [u8]) -> Option<(&'a Self, &'a [u8])> {
        let (f, rest): (&'a FlexRef<Vec<u8>>, &'a [u8]) = FlexRef::read_one(value)?;
        Some((Self::new(f), rest))
    }

    fn load(&self, store: &S) -> Result<NodeSeq<S>, S::Error> {
        Ok(if self.1.is_none() {
            NodeSeq::empty()
        } else if let Some(x) = self.1.arc_as_clone() {
            NodeSeq::from_arc_vec(x)
        } else if let Some(id) = self.1.id_as_slice() {
            NodeSeq::from_blob(store.read(id)?)
        } else {
            panic!()
        })
    }

    fn empty() -> &'static Self {
        Self::new(FlexRef::none())
    }

    fn is_empty(&self) -> bool {
        self.1.is_none()
    }

    fn manual_drop(&self) {
        self.1.manual_drop();
    }

    fn manual_clone(&self) {
        self.1.manual_clone();
    }
}

struct NodeSeq<S: BlobStore>(OwnedBlob, PhantomData<S>);

impl<S: BlobStore> NodeSeq<S> {
    fn empty() -> Self {
        Self(Blob2::empty(), PhantomData)
    }

    fn from_arc_vec(value: Arc<NodeSeqBuilder<S>>) -> Self {
        let data: &[u8] = value.as_ref().0.as_ref();
        let data: &'static [u8] = unsafe { std::mem::transmute(data) };
        Self(Blob2::owned_new(data, Some(value)), PhantomData)
    }

    fn from_blob(value: OwnedBlob) -> Self {
        Self(value, PhantomData)
    }

    fn owned_iter(&self) -> NodeSeqIter2<S> {
        NodeSeqIter2(self.0.clone(), 0, PhantomData)
    }
}

struct NodeSeqIter2<S: BlobStore>(OwnedBlob, usize, PhantomData<S>);

impl<S: BlobStore> NodeSeqIter2<S> {
    fn new(data: OwnedBlob) -> Self {
        Self(data, 0, PhantomData)
    }

    fn next(&mut self) -> Option<(Option<OwnedBlob>, TreeNode<'_, S>)> {
        if let Some(res) = TreeNode::read(&self.0[self.1..]) {
            self.1 += res.prefix().bytes().len()
                + res.value().bytes().len()
                + res.children().bytes().len();
            let v = res.value().value_opt().map(|x| self.load(x.bytes()));
            Some((v, res))
        } else {
            None
        }
    }

    fn load(&self, slice: &[u8]) -> OwnedBlob {
        self.0.slice_ref(slice)
    }
}

impl<S: BlobStore> AsRef<NodeSeqRef<S>> for NodeSeq<S> {
    fn as_ref(&self) -> &NodeSeqRef<S> {
        NodeSeqRef::new(self.0.as_ref())
    }
}

impl<S: BlobStore> Deref for NodeSeq<S> {
    type Target = NodeSeqRef<S>;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[repr(transparent)]
struct NodeSeqIter<'a, S>(&'a [u8], PhantomData<S>);

impl<'a, S: BlobStore> Clone for NodeSeqIter<'a, S> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1.clone())
    }
}

impl<'a, S: BlobStore> Copy for NodeSeqIter<'a, S> {}

impl<'a, S: BlobStore> NodeSeqIter<'a, S> {
    fn empty() -> Self {
        Self(&[], PhantomData)
    }

    fn peek(&self) -> Option<u8> {
        if self.0.is_empty() {
            None
        } else {
            TreePrefixRef::<S>::read(&self.0).map(|x| x.first())
        }
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn next(&mut self) -> Option<TreeNode<'_, S>> {
        if self.0.is_empty() {
            None
        } else {
            match TreeNode::read_one(&mut self.0) {
                Some((res, rest)) => {
                    self.0 = rest;
                    Some(res)
                }
                None => None,
            }
        }
    }
}

impl<'a, S: BlobStore> Iterator for NodeSeqIter<'a, S> {
    type Item = TreeNode<'a, S>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            None
        } else {
            match TreeNode::read_one(&mut self.0) {
                Some((res, rest)) => {
                    self.0 = rest;
                    Some(res)
                }
                None => None,
            }
        }
    }
}

struct TreeNode<'a, S: BlobStore> {
    prefix: &'a TreePrefixRef<S>,
    value: &'a TreeValueOptRef<S>,
    children: &'a TreeChildrenRef<S>,
}

impl<'a, S: BlobStore> std::fmt::Debug for TreeNode<'a, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeNode")
            .field("prefix", &self.prefix())
            .field("value", &self.value())
            .field("children", &self.children())
            .finish()
    }
}

impl<'a, S: BlobStore + 'static> TreeNode<'a, S> {
    fn prefix(&self) -> &'a TreePrefixRef<S> {
        &self.prefix
    }

    fn value(&self) -> &'a TreeValueOptRef<S> {
        &self.value
    }

    fn children(&self) -> &'a TreeChildrenRef<S> {
        &self.children
    }

    fn is_empty(&self) -> bool {
        self.children().is_empty() && self.value().is_none()
    }

    fn read(buffer: &'a [u8]) -> Option<Self> {
        Some(Self::read_one(buffer)?.0)
    }

    fn read_one(buffer: &'a [u8]) -> Option<(Self, &'a [u8])> {
        let rest = buffer;
        let (prefix, rest) = TreePrefixRef::<S>::read_one(rest)?;
        let (value, rest) = TreeValueOptRef::<S>::read_one(rest)?;
        let (children, rest) = TreeChildrenRef::<S>::read_one(rest)?;
        Some((
            Self {
                prefix,
                value,
                children,
            },
            rest,
        ))
    }

    fn manual_clone(&self) {
        self.prefix().manual_clone();
        self.value().manual_clone();
        self.children().manual_clone();
    }

    fn manual_drop(&self) {
        self.prefix().manual_drop();
        self.value().manual_drop();
        self.children().manual_drop();
    }

    fn dump(&self, indent: usize, store: &S) -> Result<(), S::Error> {
        let spacer = std::iter::repeat(" ").take(indent).collect::<String>();
        let child_ref_count = self.children().1.ref_count();
        let child_count = self.children().load(store)?.iter().count();
        println!("{}TreeNode", spacer);
        println!(
            "{}  prefix={:?} {}",
            spacer,
            self.prefix(),
            self.prefix().1.ref_count()
        );
        println!(
            "{}  value={:?} {}",
            spacer,
            self.value(),
            self.value().1.ref_count()
        );
        println!("{}  children={:?} {}", spacer, child_count, child_ref_count);
        for child in self.children().load(store)?.iter() {
            child.dump(indent + 4, store)?;
        }
        Ok(())
    }

    /// get the first value
    fn first_value(&self, store: &S) -> Result<Option<OwnedTreeValue>, S::Error> {
        Ok(if self.children().is_empty() {
            self.value().load(store)?
        } else {
            let children = self.children().load(store)?;
            children.iter().next().unwrap().first_value(store)?
        })
    }

    /// get the first entry
    fn first_entry(
        &self,
        store: &S,
        mut prefix: OwnedTreePrefix,
    ) -> Result<Option<(OwnedTreePrefix, OwnedTreeValue)>, S::Error> {
        prefix.append(&self.prefix(), store)?;
        Ok(if self.children().is_empty() {
            if let Some(value) = self.value().value_opt() {
                Some((prefix, value.load(store)?))
            } else {
                None
            }
        } else {
            let children = self.children().load(store)?;
            children.iter().next().unwrap().first_entry(store, prefix)?
        })
    }

    /// get the first value
    fn last_value(&self, store: &S) -> Result<Option<OwnedTreeValue>, S::Error> {
        Ok(if self.children().is_empty() {
            self.value().load(store)?
        } else {
            let children = self.children().load(store)?;
            children.iter().last().unwrap().first_value(store)?
        })
    }

    /// get the first entry
    fn last_entry(
        &self,
        store: &S,
        mut prefix: OwnedTreePrefix,
    ) -> Result<Option<(OwnedTreePrefix, OwnedTreeValue)>, S::Error> {
        prefix.append(&self.prefix(), store)?;
        Ok(if self.children().is_empty() {
            if let Some(value) = self.value().value_opt() {
                Some((prefix, value.load(store)?))
            } else {
                None
            }
        } else {
            let children = self.children().load(store)?;
            children.iter().last().unwrap().first_entry(store, prefix)?
        })
    }

    /// Return the subtree with the given prefix. Will return an empty tree in case there is no match.
    fn filter_prefix(&self, store: &S, prefix: &[u8]) -> Result<NodeSeqBuilder<S>, S::Error> {
        find(store, self, prefix, |x| {
            Ok(match x {
                FindResult::Found(res) => {
                    let mut b = NodeSeqBuilder::new();
                    todo!();
                    // let mut res = res.clone();
                    // res.prefix = TreePrefix::new(FlexRef::inline_or_owned_from_slice(prefix));
                    b
                }
                FindResult::Prefix { tree: res, rt } => {
                    let mut b = NodeSeqBuilder::new();
                    let rp = res.prefix().load(store)?;
                    todo!();
                    // let mut res = res.clone();
                    // res.prefix = TreePrefix::join(prefix, &rp[rp.len() - rt..]);
                    b
                }
                FindResult::NotFound { .. } => NodeSeqBuilder::empty_tree(),
            })
        })
    }
}

impl FlexRef<Vec<u8>> {
    fn arc_as_slice(&self) -> Option<&[u8]> {
        self.with_arc(|arc| {
            let t: &[u8] = arc.as_ref();
            unsafe { std::mem::transmute(t) }
        })
    }

    fn first_u8_opt(&self) -> Option<u8> {
        match self.tpe() {
            Type::None => None,
            Type::Inline => self.1.get(1).cloned(),
            Type::Arc => self.with_arc(|x| x.as_ref().get(0).cloned()).unwrap(),
            Type::Id => todo!("pack first byte into id"),
        }
    }

    fn first_u8(&self) -> u8 {
        match self.tpe() {
            Type::Inline => self.1[1],
            Type::Arc => self.with_arc(|x| x[0]).unwrap(),
            Type::None => panic!(),
            Type::Id => todo!("pack first byte into id"),
        }
    }

    fn slice(data: &[u8], target: &mut Vec<u8>) {
        if data.len() < 128 {
            target.push_inline(data);
        } else {
            target.push_arc(Arc::new(data.to_vec()));
        }
    }
}

trait VecTakeExt<T> {
    fn take(&mut self) -> Vec<T>;
}

impl<T> VecTakeExt<T> for Vec<T> {
    fn take(&mut self) -> Vec<T> {
        let mut t = Vec::new();
        std::mem::swap(&mut t, self);
        t
    }
}

impl<T> FlexRef<T> {
    const fn header(&self) -> u8 {
        self.1[0]
    }

    fn data(&self) -> &[u8] {
        let len = len(self.header());
        &self.1[1..len + 1]
    }

    fn bytes(&self) -> &[u8] {
        let len = len(self.header());
        &self.1[0..len + 1]
    }

    fn manual_drop(&self) {
        self.with_arc(|arc| unsafe {
            Arc::decrement_strong_count(Arc::as_ptr(arc));
        });
    }

    fn manual_clone(&self) {
        self.with_arc(|arc| unsafe {
            Arc::increment_strong_count(Arc::as_ptr(arc));
        });
    }

    fn new(value: &[u8]) -> &Self {
        unsafe { std::mem::transmute(value) }
    }

    fn none() -> &'static Self {
        Self::read(&[NONE]).unwrap()
    }

    fn empty() -> &'static Self {
        Self::read(&[INLINE_EMPTY]).unwrap()
    }

    fn is_empty(&self) -> bool {
        self.header() == INLINE_EMPTY
    }

    fn read_one(value: &[u8]) -> Option<(&Self, &[u8])> {
        if value.len() == 0 {
            return None;
        }
        let len = len(value[0]);
        if len + 1 > value.len() {
            return None;
        }
        Some((Self::new(value), &value[len + 1..]))
    }

    fn read(value: &[u8]) -> Option<&Self> {
        Some(Self::read_one(value)?.0)
    }

    const fn tpe(&self) -> Type {
        tpe(self.header())
    }

    fn inline_as_ref(&self) -> Option<&[u8]> {
        if let Type::Inline = self.tpe() {
            Some(self.data())
        } else {
            None
        }
    }

    fn arc_as_clone(&self) -> Option<Arc<T>> {
        self.with_arc(|x| x.clone())
    }

    fn id_as_slice(&self) -> Option<&[u8]> {
        self.with_id(|_| todo!())
    }

    fn ref_count(&self) -> usize {
        self.with_arc(|x| Arc::strong_count(x)).unwrap_or_default()
    }

    fn with_arc<U>(&self, f: impl Fn(&Arc<T>) -> U) -> Option<U> {
        if self.tpe() == Type::Arc {
            let value = u64::from_be_bytes(self.1[1..9].try_into().unwrap());
            let value = usize::try_from(value).unwrap();
            let arc: Arc<T> = unsafe { std::mem::transmute(value) };
            let res = Some(f(&arc));
            std::mem::forget(arc);
            res
        } else {
            None
        }
    }

    fn with_inline<U>(&self, f: impl Fn(&[u8]) -> U) -> Option<U> {
        if self.tpe() == Type::Inline {
            Some(f(self.data()))
        } else {
            None
        }
    }

    fn with_id<U>(&self, f: impl Fn(u64) -> U) -> Option<U> {
        if self.tpe() == Type::Id {
            let id = u64::from_be_bytes(self.1[1..9].try_into().unwrap());
            Some(f(id))
        } else {
            None
        }
    }

    fn is_none(&self) -> bool {
        self.tpe() == Type::None
    }

    fn is_some(&self) -> bool {
        !self.is_some()
    }

    fn copy_to(&self, target: &mut Vec<u8>) {
        self.with_arc(|arc| {
            std::mem::forget(arc.clone());
        });
        target.extend_from_slice(self.bytes());
    }
}

const fn len(value: u8) -> usize {
    (value & 0x3f) as usize
}

const fn tpe(value: u8) -> Type {
    match value >> 6 {
        0 => Type::None,
        1 => Type::Inline,
        2 => Type::Arc,
        3 => Type::Id,
        _ => panic!(),
    }
}

const fn make_header_byte(tpe: Type, len: usize) -> u8 {
    assert!(len < 64);
    (len as u8)
        | match tpe {
            Type::None => 0,
            Type::Inline => 1,
            Type::Arc => 2,
            Type::Id => 3,
        } << 6
}

const NONE: u8 = make_header_byte(Type::None, 0);
const INLINE_EMPTY: u8 = make_header_byte(Type::Inline, 0);
const ARC8: u8 = make_header_byte(Type::Arc, 8);

#[repr(transparent)]
struct NodeSeqRef<S: BlobStore>(PhantomData<S>, [u8]);

impl<S: BlobStore> NodeSeqRef<S> {
    fn new(value: &[u8]) -> &Self {
        unsafe { std::mem::transmute(value) }
    }

    fn iter(&self) -> NodeSeqIter<'_, S> {
        NodeSeqIter(&self.1, PhantomData)
    }

    fn find(&self, first: u8) -> Option<TreeNode<'_, S>> {
        // todo: optimize
        for leaf in self.iter() {
            let first_opt = leaf.prefix().first_opt();
            if first_opt == Some(first) {
                // found it
                return Some(leaf);
            } else if first_opt > Some(first) {
                // not going to come anymore
                return None;
            }
        }
        None
    }
}

#[derive(Default)]

struct InPlaceFlexRefSeqBuilder {
    // place for the data. [..t1] and [s1..] contain valid sequences of flexrefs.
    // The rest is to be considered uninitialized
    vec: Vec<u8>,
    // end of the result
    t1: usize,
    // start of the source
    s0: usize,
}

impl fmt::Debug for InPlaceFlexRefSeqBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}, {}]",
            Hex::new(self.target_slice()),
            Hex::new(self.source_slice())
        )
    }
}

trait Extendable {
    fn reserve(&mut self, n: usize);

    fn extend_from_slice(&mut self, data: &[u8]);

    fn push(&mut self, value: u8);

    fn push_id(&mut self, id: &[u8]) {
        self.reserve(1 + id.len());
        self.push(make_header_byte(Type::Id, id.len()));
        self.extend_from_slice(id);
    }

    fn push_none(&mut self) {
        self.push(NONE);
    }

    fn push_arc<T>(&mut self, arc: Arc<T>) {
        let data: usize = unsafe { std::mem::transmute(arc) };
        let data: u64 = data as u64;
        self.reserve(9);
        self.push(ARC8);
        self.extend_from_slice(&data.to_be_bytes());
    }

    fn push_arc_or_inline(&mut self, data: impl AsRef<[u8]>) {
        let data = data.as_ref();
        if data.len() < 64 {
            self.push_inline(data.as_ref())
        } else {
            self.push_arc(Arc::new(data.to_vec()))
        }
    }

    fn push_inline(&mut self, data: &[u8]) {
        debug_assert!(data.len() < 64);
        self.reserve(data.len() + 1);
        self.push(make_header_byte(Type::Inline, data.len()));
        self.extend_from_slice(data);
    }
}

impl Extendable for Vec<u8> {
    fn reserve(&mut self, n: usize) {
        let free = self.capacity() - self.len();
        if free < n {
            self.reserve(n - free);
        }
    }

    fn push(&mut self, value: u8) {
        self.push(value)
    }

    fn extend_from_slice(&mut self, data: &[u8]) {
        self.extend_from_slice(data)
    }
}

impl Extendable for InPlaceFlexRefSeqBuilder {
    fn reserve(&mut self, n: usize) {
        let gap = self.gap();
        if gap < n {
            let missing = n - gap;
            self.vec.reserve(missing);
            let space = self.vec.capacity() - self.vec.len();
            self.vec
                .splice(self.s0..self.s0, std::iter::repeat(0).take(space));
            self.s0 += space;
        }
    }

    fn push(&mut self, value: u8) {
        self.reserve(1);
        self.vec[self.t1] = value;
        self.t1 += 1;
    }

    fn extend_from_slice(&mut self, data: &[u8]) {
        let len = data.len();
        self.reserve(len);
        self.vec[self.t1..self.t1 + len].copy_from_slice(data);
        self.t1 += len;
    }
}

fn validate_flexref_slice(value: &[u8]) -> usize {
    let mut iter = FlexRefIter(value);
    let mut n = 0;
    while let Some(item) = iter.next() {
        // debug_assert!(!item.is_empty());
        n += 1;
    }
    // debug_assert!(iter.is_empty());
    n
}

fn validate_nodeseq_slice<S: BlobStore>(value: &[u8]) -> usize {
    let mut iter = NodeSeqIter::<S>(value, PhantomData);
    let mut n = 0;
    while let Some(item) = iter.next() {
        debug_assert!(item.prefix().is_valid());
        debug_assert!(item.value().is_valid());
        debug_assert!(item.children().is_valid());
        n += 1;
    }
    debug_assert!(iter.is_empty());
    n
}

impl InPlaceFlexRefSeqBuilder {
    /// Create a new builder by taking over the data from a vec
    pub fn new(vec: Vec<u8>) -> Self {
        Self { vec, t1: 0, s0: 0 }
    }

    fn into_inner(mut self) -> Vec<u8> {
        // debug_assert!(self.source_count() == 0);
        // debug_assert!(self.target_count() % 3 == 0);
        self.vec.truncate(self.t1);
        self.vec
    }

    fn take_result(&mut self) -> Vec<u8> {
        // debug_assert!(self.source_count() == 0);
        // debug_assert!(self.target_count() % 3 == 0);
        self.vec.truncate(self.t1);
        let mut res = Vec::new();
        std::mem::swap(&mut self.vec, &mut res);
        self.t1 = 0;
        self.s0 = 0;
        res
    }

    pub fn source_count(&self) -> usize {
        validate_flexref_slice(self.source_slice())
    }

    pub fn target_count(&self) -> usize {
        validate_flexref_slice(self.source_slice())
    }

    pub fn total_count(&self) -> usize {
        self.source_count() + self.target_count()
    }

    fn drop(&mut self, n: usize) {
        // debug_assert!(n <= self.source_slice().len());
        self.s0 += n;
    }

    fn forward_all(&mut self) {
        self.forward(self.source_slice().len())
    }

    fn rewind_all(&mut self) {
        self.rewind(self.target_slice().len())
    }

    fn forward(&mut self, n: usize) {
        // debug_assert!(n <= self.source_slice().len());
        if self.gap() == 0 {
            self.t1 += n;
            self.s0 += n;
        } else {
            self.vec.copy_within(self.s0..self.s0 + n, self.t1);
            self.t1 += n;
            self.s0 += n;
        }
    }

    fn rewind(&mut self, to: usize) {
        // debug_assert!(to <= self.t1);
        let n = self.t1 - to;
        if self.gap() == 0 {
            self.t1 -= n;
            self.s0 -= n;
        } else {
            self.vec.copy_within(to..to + n, self.s0 - n);
            self.t1 -= n;
            self.s0 -= n;
        }
    }

    #[inline]
    const fn gap(&self) -> usize {
        self.s0 - self.t1
    }

    #[inline]
    fn target_slice(&self) -> &[u8] {
        &self.vec[..self.t1]
    }

    #[inline]
    fn source_slice(&self) -> &[u8] {
        &self.vec[self.s0..]
    }

    #[inline]
    fn has_remaining(&self) -> bool {
        self.s0 < self.vec.len()
    }
}

struct FlexRefIter<'a>(&'a [u8]);

impl<'a> FlexRefIter<'a> {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a> Iterator for FlexRefIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            None
        } else {
            let len = 1 + len(self.0[0]);
            if self.0.len() >= len {
                let res = Some(&self.0[..len]);
                self.0 = &self.0[len..];
                res
            } else {
                None
            }
        }
    }
}

#[derive(Debug)]
#[repr(transparent)]
struct BlobOfFlexRef(Blob);

impl BlobOwner for BlobOfFlexRef {
    fn get_slice(&self, offset: usize) -> &[u8] {
        let len = 1 + len(self.0[offset]);
        &self.0[offset..offset + len]
    }
}

#[derive(Debug)]
struct FlexRefIter2(Blob, usize);

impl FlexRefIter2 {
    fn next(&mut self) -> Option<&[u8]> {
        if self.0.is_empty() {
            None
        } else {
            let data = self.0.as_ref();
            let offset = self.1;
            let len = 1 + len(data[offset]);
            if self.0.len() >= len {
                self.1 += len;
                Some(&data[offset..offset + len])
            } else {
                None
            }
        }
    }
}

#[derive(Debug)]
struct AtPrefix;
#[derive(Debug)]
struct AtValue;
#[derive(Debug)]
struct AtChildren;

trait IterPosition: Debug {}
impl IterPosition for AtPrefix {}
impl IterPosition for AtValue {}
impl IterPosition for AtChildren {}

macro_rules! unwrap_or_break {
    ($res:expr) => {
        match $res {
            Some(val) => val,
            None => {
                break;
            }
        }
    };
}

#[derive(Debug)]
#[repr(transparent)]
struct InPlaceBuilderRef<'a, S: BlobStore, P: IterPosition>(
    &'a mut InPlaceFlexRefSeqBuilder,
    PhantomData<(S, P)>,
);

impl<'a, S: BlobStore> InPlaceBuilderRef<'a, S, AtPrefix> {
    pub fn peek(&self) -> &TreePrefixRef<S> {
        TreePrefixRef::new(FlexRef::new(self.0.source_slice()))
    }

    pub fn move_prefix(self) -> InPlaceBuilderRef<'a, S, AtValue> {
        let len = self.peek().bytes().len();
        self.0.forward(len);
        self.done()
    }

    pub fn push_prefix_ref(
        mut self,
        prefix: impl AsRef<[u8]>,
    ) -> InPlaceBuilderRef<'a, S, AtValue> {
        self.drop_current();
        self.0.push_arc_or_inline(prefix.as_ref());
        self.done()
    }

    pub fn insert_converted<S2: BlobStore>(
        mut self,
        prefix: &TreePrefixRef<S2>,
        store: &S2,
    ) -> Result<InPlaceBuilderRef<'a, S, AtValue>, S::Error> {
        // todo fix
        self.0.extend_from_slice(prefix.bytes());
        Ok(self.done())
    }

    fn drop_current(&mut self) {
        let peek = self.peek();
        let len = peek.bytes().len();
        peek.manual_drop();
        self.0.drop(len);
    }

    fn mark(&self) -> usize {
        self.0.t1
    }

    fn rewind(self, to: usize) -> InPlaceBuilderRef<'a, S, AtPrefix> {
        self.0.rewind(to);
        InPlaceBuilderRef(self.0, PhantomData)
    }

    fn done(self) -> InPlaceBuilderRef<'a, S, AtValue> {
        InPlaceBuilderRef(self.0, PhantomData)
    }
}

impl<'a, S: BlobStore> InPlaceBuilderRef<'a, S, AtValue> {
    fn done(self) -> InPlaceBuilderRef<'a, S, AtChildren> {
        InPlaceBuilderRef(self.0, PhantomData)
    }

    fn drop_current(&mut self) {
        let len = self.peek().bytes().len();
        self.peek().manual_drop();
        self.0.drop(len);
    }

    pub fn peek(&self) -> &TreeValueOptRef<S> {
        TreeValueOptRef::new(FlexRef::new(self.0.source_slice()))
    }

    pub fn move_value(self) -> InPlaceBuilderRef<'a, S, AtChildren> {
        let len = self.peek().bytes().len();
        self.0.forward(len);
        self.done()
    }

    fn push_value_none(mut self) -> InPlaceBuilderRef<'a, S, AtChildren> {
        self.drop_current();
        self.0.push_none();
        self.done()
    }

    pub fn push_value_opt(self, value: Option<TreeValue>) -> InPlaceBuilderRef<'a, S, AtChildren> {
        if let Some(value) = value {
            self.push_value(value)
        } else {
            self.push_value_none()
        }
    }

    fn push_value(mut self, value: TreeValue) -> InPlaceBuilderRef<'a, S, AtChildren> {
        self.drop_current();
        self.0.push_arc_or_inline(value);
        self.done()
    }

    pub fn push_converted<S2: BlobStore>(
        mut self,
        value: &TreeValueOptRef<S2>,
        store: &S2,
    ) -> Result<InPlaceBuilderRef<'a, S, AtChildren>, S::Error> {
        self.drop_current();
        self.insert_converted(value, store)
    }

    pub fn insert_converted<S2: BlobStore>(
        self,
        value: &TreeValueOptRef<S2>,
        store: &S2,
    ) -> Result<InPlaceBuilderRef<'a, S, AtChildren>, S::Error> {
        value.manual_clone();
        self.0.extend_from_slice(value.bytes());
        Ok(self.done())
    }
}

// use lazy_static::lazy_static;

// lazy_static! {
//    static ref MUTATE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::default();
//    static ref GROW_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::default();
// }

impl<'a, S: BlobStore> InPlaceBuilderRef<'a, S, AtChildren> {
    /// peek the child at the current position as a TreeChildrenRef
    ///
    /// this returns an error in case the item extends behind the end of the buffer
    pub fn peek(&self) -> &TreeChildrenRef<S> {
        TreeChildrenRef::new(FlexRef::new(self.0.source_slice()))
    }

    pub fn mutate(
        mut self,
        store: &S,
        f: impl Fn(&mut InPlaceNodeSeqBuilder<S>) -> Result<(), S::Error>,
    ) -> Result<InPlaceBuilderRef<'a, S, AtPrefix>, S::Error> {
        // MUTATE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut arc = self.take_arc(store)?;
        let mut values = Arc::make_mut(&mut arc);
        let mut builder = InPlaceNodeSeqBuilder::<S>::new(&mut values);
        match f(&mut builder) {
            Ok(_) => {
                // todo: don't do this always?
                builder.inner.rewind_all();
                builder.canonicalize_all();
                *values = builder.into_inner();
                Ok(if !values.is_empty() {
                    self.push_new_arc(arc)
                } else {
                    self.push_empty()
                })
            }
            Err(cause) => {
                self.move_children();
                Err(cause)
            }
        }
    }

    fn take_arc(&mut self, store: &S) -> Result<Arc<NodeSeqBuilder<S>>, S::Error> {
        let v = self.peek();
        let len = v.bytes().len();
        let res = if let Some(arc) = v.1.arc_as_clone() {
            v.manual_drop();
            arc
        } else {
            // todo: load data if needed
            Arc::new(NodeSeqBuilder::new())
        };
        // replace current value with "no children" paceholder
        self.0.s0 += len - 1;
        self.0.vec[self.0.s0] = NONE;
        Ok(res)
    }

    pub fn set_new_arc(
        mut self,
        value: Arc<NodeSeqBuilder<S>>,
    ) -> InPlaceBuilderRef<'a, S, AtChildren> {
        debug_assert!(Arc::strong_count(&value) == 1);
        let t1 = self.0.t1;
        self.drop_current();
        self.0.push_arc(value);
        self.0.rewind(t1);
        InPlaceBuilderRef(self.0, PhantomData)
    }

    fn drop_current(&mut self) {
        let peek = self.peek();
        let len = peek.bytes().len();
        peek.manual_drop();
        self.0.drop(len);
    }

    pub fn push_new_arc(
        mut self,
        value: Arc<NodeSeqBuilder<S>>,
    ) -> InPlaceBuilderRef<'a, S, AtPrefix> {
        debug_assert!(Arc::strong_count(&value) == 1);
        self.drop_current();
        self.0.push_arc(value);
        self.done()
    }

    pub fn push_empty(mut self) -> InPlaceBuilderRef<'a, S, AtPrefix> {
        self.drop_current();
        self.0.push_none();
        self.done()
    }

    fn done(self) -> InPlaceBuilderRef<'a, S, AtPrefix> {
        InPlaceBuilderRef(self.0, PhantomData)
    }

    pub fn move_children(self) -> InPlaceBuilderRef<'a, S, AtPrefix> {
        self.0.forward(self.peek().bytes().len());
        self.done()
    }

    pub fn insert_converted<S2: BlobStore>(
        self,
        children: &TreeChildrenRef<S2>,
        store: &S2,
    ) -> Result<InPlaceBuilderRef<'a, S, AtPrefix>, S::Error> {
        // todo fix
        children.manual_clone();
        self.0.extend_from_slice(children.bytes());
        Ok(self.done())
    }
}

#[repr(transparent)]
struct InPlaceNodeSeqBuilder<S: BlobStore = NoStore> {
    inner: InPlaceFlexRefSeqBuilder,
    p: PhantomData<S>,
}

impl<S: BlobStore> InPlaceNodeSeqBuilder<S> {
    /// creates an InPlaceNodeSeqBuilder by ripping the inner vec out of a NodeSeqBuilder
    fn new(from: &mut NodeSeqBuilder<S>) -> Self {
        Self {
            inner: InPlaceFlexRefSeqBuilder::new(from.0.take()),
            p: PhantomData,
        }
    }

    /// consumes an InPlaceNodeSeqBuilder by storing the result in a NodeSeqBuilder
    fn into_inner(mut self) -> NodeSeqBuilder<S> {
        let t = self.inner.take_result();
        // validate_nodeseq_slice::<S>(&t);
        NodeSeqBuilder(t, PhantomData)
    }

    /// A cursor, assuming we are currently at the start of a triple
    #[inline]
    fn cursor(&mut self) -> InPlaceBuilderRef<'_, S, AtPrefix> {
        InPlaceBuilderRef(&mut self.inner, PhantomData)
    }

    /// Peek element of the source
    fn peek(&self) -> Option<u8> {
        if !self.inner.has_remaining() {
            None
        } else {
            Some(TreePrefixRef::<S>::new(FlexRef::new(self.inner.source_slice())).first())
        }
    }

    /// move one triple from the source to the target
    fn move_one(&mut self) {
        self.cursor().move_prefix().move_value().move_children();
    }

    fn move_all(&mut self) {
        while self.inner.has_remaining() {
            self.move_one()
        }
    }

    /// move one triple from the source to the target, canonicalizing
    fn canonicalize_one(&mut self) {
        let p = self.cursor();
        let start = p.mark();
        let pe = p.peek().is_empty();
        let v = p.move_prefix();
        let ve = v.peek().is_none();
        let c = v.move_value();
        let ce = c.peek().is_empty();
        let p1 = c.move_children();
        if ve && ce && !pe {
            p1.rewind(start)
                .push_prefix_ref(&[])
                .push_value_none()
                .push_empty();
        }
    }

    fn rewind_all(&mut self) {
        self.inner.rewind_all();
    }

    fn canonicalize_all(&mut self) {
        while self.inner.has_remaining() {
            self.canonicalize_one()
        }
    }

    /// converts and inserts! a tree node, without dropping anything
    fn insert_converted<S2: BlobStore>(
        &mut self,
        node: TreeNode<S2>,
        store: &S2,
    ) -> Result<(), S::Error> {
        // todo: we must not fail in the middle, since that will leave a mess. Hence the unwrap. Fix this.
        self.cursor()
            .insert_converted(node.prefix(), store)
            .unwrap()
            .insert_converted(node.value(), store)
            .unwrap()
            .insert_converted(node.children(), store)
            .unwrap();
        Ok(())
    }
}

impl<S: BlobStore> Drop for InPlaceNodeSeqBuilder<S> {
    fn drop(&mut self) {
        let mut target = FlexRefIter(self.inner.target_slice());
        let mut i = 0;
        while let Some(x) = target.next() {
            match i % 3 {
                0 => TreePrefixRef::<S>::new(FlexRef::new(x)).manual_drop(),
                1 => TreeValueOptRef::<S>::new(FlexRef::new(x)).manual_drop(),
                2 => TreeChildrenRef::<S>::new(FlexRef::new(x)).manual_drop(),
                _ => panic!(),
            }
        }
        if !target.0.is_empty() {
            return;
        }
        let mut source = FlexRefIter(self.inner.source_slice());
        while let Some(x) = source.next() {
            match i % 3 {
                0 => TreePrefixRef::<S>::new(FlexRef::new(x)).manual_drop(),
                1 => TreeValueOptRef::<S>::new(FlexRef::new(x)).manual_drop(),
                2 => TreeChildrenRef::<S>::new(FlexRef::new(x)).manual_drop(),
                _ => panic!(),
            }
        }
        if !source.0.is_empty() {
            return;
        }
    }
}

struct TreeNodeMut<'a, S: BlobStore = NoStore>(&'a mut InPlaceNodeSeqBuilder<S>, TreeNode<'a, S>);

impl<'a, S: BlobStore> AsRef<TreeNode<'a, S>> for TreeNodeMut<'a, S> {
    fn as_ref(&self) -> &TreeNode<'a, S> {
        &self.1
    }
}

impl<'a, S: BlobStore> Deref for TreeNodeMut<'a, S> {
    type Target = TreeNode<'a, S>;

    fn deref(&self) -> &TreeNode<'a, S> {
        &self.as_ref()
    }
}

struct NodeSeqBuilder<S: BlobStore = NoStore>(Vec<u8>, PhantomData<S>);

impl<S: BlobStore> Debug for NodeSeqBuilder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<S: BlobStore> BlobOwner for NodeSeqBuilder<S> {
    fn get_slice(&self, _: usize) -> &[u8] {
        self.0.as_ref()
    }
}

impl<S: BlobStore> NodeSeqBuilder<S> {
    fn as_owned_blob(self: Arc<Self>) -> OwnedBlob {
        let data: &[u8] = self.0.as_ref();
        let data: &'static [u8] = unsafe { std::mem::transmute(data) };
        OwnedBlob::owned_new(data, Some(self))
    }

    fn new() -> Self {
        Self(Vec::with_capacity(64), PhantomData)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn as_blob(self: Arc<NodeSeqBuilder<S>>) -> Blob {
        Blob::new(self, 0).unwrap()
    }

    fn into_inner(mut self) -> Vec<u8> {
        let mut r = Vec::new();
        std::mem::swap(&mut self.0, &mut r);
        drop(self);
        r
    }

    fn push_prefix(&mut self, prefix: impl AsRef<[u8]>) {
        self.0.push_arc_or_inline(prefix);
    }

    fn push_value(&mut self, value: Option<TreeValue>) {
        if let Some(value) = value {
            self.0.push_arc_or_inline(value);
        } else {
            self.0.push_none()
        }
    }

    fn push_value_ref(&mut self, value: &TreeValueOptRef<S>) {
        value.manual_clone();
        self.0.extend_from_slice(value.bytes());
    }

    fn push_children(&mut self, value: NodeSeqBuilder<S>) {
        if !value.0.is_empty() {
            self.0.push_arc(Arc::new(value))
        } else {
            self.0.push_none()
        }
    }

    fn push_children_ref(&mut self, value: &TreeChildrenRef<S>) {
        value.manual_clone();
        self.0.extend_from_slice(value.bytes());
    }

    fn empty_tree() -> Self {
        let mut res = InPlaceFlexRefSeqBuilder::default();
        res.push_arc_or_inline(&[]);
        res.push_none();
        res.push_none();
        Self(res.into_inner(), PhantomData)
    }

    fn push_new(
        &mut self,
        prefix: TreePrefix,
        value: Option<TreeValue>,
        children: NodeSeqBuilder<S>,
    ) {
        self.push_prefix(prefix);
        self.push_value(value);
        self.push_children(children);
    }

    fn push_detached<S2: BlobStore>(
        &mut self,
        node: TreeNode<'_, S2>,
        store: &S2,
    ) -> Result<(), S2::Error> {
        /// TODO: get rid of this!
        let t: &mut NodeSeqBuilder<S2> = unsafe { std::mem::transmute(self) };
        Ok(t.push(node))
    }

    fn push_new_unsplit(
        &mut self,
        prefix: TreePrefix,
        value: Option<TreeValue>,
        children: NodeSeqBuilder<S>,
        store: &S,
    ) -> Result<(), S::Error> {
        // todo
        Ok(self.push_new(prefix, value, children))
    }

    fn push(&mut self, node: TreeNode<'_, S>) {
        node.prefix().1.copy_to(&mut self.0);
        node.value().1.copy_to(&mut self.0);
        node.children().1.copy_to(&mut self.0);
    }

    fn push_shortened(
        &mut self,
        node: &TreeNode<'_, S>,
        store: &S,
        n: usize,
    ) -> Result<(), S::Error> {
        if n > 0 {
            let prefix = node.prefix().load2(store)?;
            assert!(n < prefix.len());
            FlexRef::slice(&prefix[n..], &mut self.0);
        } else {
            node.prefix().1.copy_to(&mut self.0);
        }
        node.value().1.copy_to(&mut self.0);
        node.children().1.copy_to(&mut self.0);
        Ok(())
    }

    fn push_converted<S2: BlobStore>(
        &mut self,
        node: TreeNode<'_, S2>,
        store: &S2,
    ) -> Result<(), S2::Error> {
        /// TODO: get rid of this!
        let t: &mut NodeSeqBuilder<S2> = unsafe { std::mem::transmute(self) };
        Ok(t.push(node))
    }

    fn push_shortened_converted<S2: BlobStore>(
        &mut self,
        node: &TreeNode<'_, S2>,
        store: &S2,
        n: usize,
    ) -> Result<(), S2::Error> {
        /// TODO: get rid of this!
        let t: &mut NodeSeqBuilder<S2> = unsafe { std::mem::transmute(self) };
        t.push_shortened(node, store, n)
    }

    fn shortened(node: &TreeNode<'_, S>, store: &S, n: usize) -> Result<Self, S::Error> {
        let mut res = Self(Vec::new(), PhantomData);
        res.push_shortened(node, store, n)?;
        Ok(res)
    }

    fn single(key: &[u8], value: &[u8]) -> Self {
        let mut t = InPlaceFlexRefSeqBuilder::default();
        t.push_arc_or_inline(key);
        t.push_arc_or_inline(value);
        t.push_none();
        Self(t.into_inner(), PhantomData)
    }
}

impl<S: BlobStore> AsRef<NodeSeqRef<S>> for NodeSeqBuilder<S> {
    fn as_ref(&self) -> &NodeSeqRef<S> {
        NodeSeqRef::new(self.0.as_ref())
    }
}

impl<S: BlobStore> Deref for NodeSeqBuilder<S> {
    type Target = NodeSeqRef<S>;

    fn deref(&self) -> &Self::Target {
        NodeSeqRef::new(self.0.as_ref())
    }
}

impl<S: BlobStore> Clone for NodeSeqBuilder<S> {
    fn clone(&self) -> Self {
        for elem in self.as_ref().iter() {
            elem.manual_clone();
        }
        Self(self.0.clone(), PhantomData)
    }
}

impl<S: BlobStore> Drop for NodeSeqBuilder<S> {
    fn drop(&mut self) {
        for elem in self.as_ref().iter() {
            elem.manual_drop();
        }
        self.0.truncate(0);
    }
}

#[derive(Debug, Clone)]
pub struct Tree<S: BlobStore = NoStore> {
    /// This contains exactly one node, even in the case of an empty tree
    node: NodeSeqBuilder<S>,
    /// The associated store
    store: S,
}

impl Tree {
    fn empty() -> Self {
        Self {
            node: NodeSeqBuilder::empty_tree(),
            store: NoStore,
        }
    }

    fn single(key: &[u8], value: &[u8]) -> Self {
        Self {
            node: NodeSeqBuilder::single(key, value),
            store: NoStore,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (IterKey, TreeValueRefWrapper)> {
        self.try_iter().map(unwrap_safe)
    }

    pub fn values(&self) -> impl Iterator<Item = TreeValueRefWrapper> {
        self.try_values().map(unwrap_safe)
    }

    pub fn get(&self, key: &[u8]) -> Option<TreeValue> {
        unwrap_safe(self.try_get(key))
    }

    pub fn contains_key(&self, key: &[u8]) -> bool {
        unwrap_safe(self.try_contains_key(key))
    }

    pub fn first_value(&self) -> Option<TreeValue> {
        unwrap_safe(self.try_first_value())
    }

    pub fn last_value(&self) -> Option<TreeValue> {
        unwrap_safe(self.try_last_value())
    }

    pub fn first_entry(&self, prefix: OwnedTreePrefix) -> Option<(TreePrefix, TreeValue)> {
        unwrap_safe(self.try_first_entry(prefix))
    }

    pub fn last_entry(&self, prefix: OwnedTreePrefix) -> Option<(TreePrefix, TreeValue)> {
        unwrap_safe(self.try_last_entry(prefix))
    }

    pub fn outer_combine(
        &self,
        that: &Tree,
        f: impl Fn(&TreeValueRef, &TreeValueRef) -> Option<OwnedTreeValue> + Copy,
    ) -> Tree {
        unwrap_safe(self.try_outer_combine::<NoStore, NoError, _>(that, |a, b| Ok(f(a, b))))
    }

    pub fn outer_combine_with(
        &mut self,
        that: &Tree,
        f: impl Fn(&TreeValueRef, &TreeValueRef) -> Option<OwnedTreeValue> + Copy,
    ) {
        unwrap_safe(self.try_outer_combine_with::<NoStore, _>(that, |a, b| Ok(f(a, b))))
    }
}

impl<S: BlobStore + Clone> Tree<S> {
    fn node(&self) -> TreeNode<'_, S> {
        TreeNode::read(&self.node.0).unwrap()
    }

    fn dump(&self) -> Result<(), S::Error> {
        self.node().dump(0, &self.store)
    }

    pub fn try_iter(&self) -> Iter<S> {
        let iter = NodeSeqIter2::new(Arc::new(self.node.clone()).as_owned_blob());
        Iter::new(iter, self.store.clone(), IterKey::default())
    }

    pub fn try_values(&self) -> Values<S> {
        let iter = NodeSeqIter2::new(Arc::new(self.node.clone()).as_owned_blob());
        Values::new(iter, self.store.clone())
    }

    /// Get the value for a given key
    fn try_get(&self, key: &[u8]) -> Result<Option<OwnedTreeValue>, S::Error> {
        // if we find a tree at exactly the location, and it has a value, we have a hit
        find(&self.store, &self.node(), key, |r| {
            Ok(if let FindResult::Found(tree) = r {
                tree.value().load(&self.store)?
            } else {
                None
            })
        })
    }

    /// True if key is contained in this set
    fn try_contains_key(&self, key: &[u8]) -> Result<bool, S::Error> {
        // if we find a tree at exactly the location, and it has a value, we have a hit
        find(&self.store, &self.node(), key, |r| {
            Ok(if let FindResult::Found(tree) = r {
                tree.value().is_some()
            } else {
                false
            })
        })
    }

    pub fn try_outer_combine<S2, E, F>(&self, that: &Tree<S2>, f: F) -> Result<Tree, E>
    where
        S2: BlobStore,
        E: From<S::Error> + From<S2::Error> + From<NoError>,
        F: Fn(&TreeValueRef<S>, &TreeValueRef<S2>) -> Result<Option<OwnedTreeValue>, E> + Copy,
    {
        let mut nodes = NodeSeqBuilder::new();
        outer_combine(
            &self.node.iter().next().unwrap(),
            &self.store,
            &that.node.iter().next().unwrap(),
            &that.store,
            f,
            &mut nodes,
        )?;
        Ok(Tree {
            node: nodes,
            store: NoStore,
        })
    }

    pub fn try_outer_combine_with<S2, F>(&mut self, that: &Tree<S2>, f: F) -> Result<(), S::Error>
    where
        S2: BlobStore,
        S::Error: From<S2::Error> + From<NoError>,
        F: Fn(&TreeValueRef<S>, &TreeValueRef<S2>) -> Result<Option<OwnedTreeValue>, S::Error>
            + Copy,
    {
        let mut iter = InPlaceNodeSeqBuilder::new(&mut self.node);
        outer_combine_with(
            iter.cursor(),
            &self.store,
            &that.node.iter().next().unwrap(),
            &that.store,
            f,
        )?;
        self.node = iter.into_inner();
        Ok(())
    }

    pub fn try_first_value(&self) -> Result<Option<OwnedTreeValue>, S::Error> {
        self.node().first_value(&self.store)
    }

    pub fn try_last_value(&self) -> Result<Option<OwnedTreeValue>, S::Error> {
        self.node().last_value(&self.store)
    }

    pub fn try_first_entry(
        &self,
        prefix: OwnedTreePrefix,
    ) -> Result<Option<(OwnedTreePrefix, OwnedTreeValue)>, S::Error> {
        self.node().first_entry(&self.store, prefix)
    }

    pub fn try_last_entry(
        &self,
        prefix: OwnedTreePrefix,
    ) -> Result<Option<(OwnedTreePrefix, OwnedTreeValue)>, S::Error> {
        self.node().last_entry(&self.store, prefix)
    }

    pub fn try_filter_prefix<'a>(&'a self, prefix: &[u8]) -> Result<Tree<S>, S::Error> {
        self.node()
            .filter_prefix(&self.store, prefix)
            .map(|node| Tree {
                node,
                store: self.store.clone(),
            })
    }
}

// common prefix of two slices.
fn common_prefix<'a, T: Eq>(a: &'a [T], b: &'a [T]) -> usize {
    a.iter().zip(b).take_while(|(a, b)| a == b).count()
}

fn outer_combine<A, B, E, F>(
    a: &TreeNode<A>,
    ab: &A,
    b: &TreeNode<B>,
    bb: &B,
    f: F,
    target: &mut NodeSeqBuilder,
) -> Result<(), E>
where
    A: BlobStore,
    B: BlobStore,
    E: From<A::Error> + From<B::Error> + From<NoError>,
    F: Fn(&TreeValueRef<A>, &TreeValueRef<B>) -> Result<Option<OwnedTreeValue>, E> + Copy,
{
    let ap = a.prefix().load2(ab)?;
    let bp = b.prefix().load2(bb)?;
    let n = common_prefix(ap.as_ref(), bp.as_ref());
    let prefix = TreePrefix::from_slice(&ap[..n]);
    let mut children: NodeSeqBuilder = NodeSeqBuilder::new();
    let value: Option<TreeValue>;
    if n == ap.len() && n == bp.len() {
        // prefixes are identical
        value = match (a.value().value_opt(), b.value().value_opt()) {
            (Some(a), Some(b)) => f(a, b)?,
            (Some(a), None) => Some(a.load2(ab)?),
            (None, Some(b)) => Some(b.load2(bb)?),
            (None, None) => None,
        };
        let ac = a.children().load(ab)?;
        let bc = b.children().load(bb)?;
        children = outer_combine_children(ac.iter(), &ab, bc.iter(), &bb, f)?;
    } else if n == ap.len() {
        // a is a prefix of b
        // value is value of a
        value = a.value().load(ab)?;
        let ac = a.children().load(ab)?;
        let bc = NodeSeqBuilder::shortened(b, bb, n)?;
        children = outer_combine_children(ac.iter(), &ab, bc.iter(), &bb, f)?;
    } else if n == bp.len() {
        // b is a prefix of a
        // value is value of b
        value = b.value().load(bb)?;
        let ac = NodeSeqBuilder::shortened(a, ab, n)?;
        let bc = b.children().load(bb)?;
        children = outer_combine_children(ac.iter(), &ab, bc.iter(), &bb, f)?;
    } else {
        // the two nodes are disjoint
        // value is none
        value = None;
        // children is just the shortened children a and b in the right order
        if ap[n] <= bp[n] {
            children.push_shortened_converted(a, ab, n)?;
            children.push_shortened_converted(b, bb, n)?;
        } else {
            children.push_shortened_converted(b, bb, n)?;
            children.push_shortened_converted(a, ab, n)?;
        }
    }
    target.push_new_unsplit(prefix, value, children, &NoStore)?;
    Ok(())
}

fn outer_combine_children<'a, A, B, E, F>(
    a: NodeSeqIter<'a, A>,
    ab: &A,
    b: NodeSeqIter<'a, B>,
    bb: &B,
    f: F,
) -> Result<NodeSeqBuilder, E>
where
    A: BlobStore,
    B: BlobStore,
    E: From<A::Error> + From<B::Error> + From<NoError>,
    F: Fn(&TreeValueRef<A>, &TreeValueRef<B>) -> Result<Option<OwnedTreeValue>, E> + Copy,
{
    let mut res = NodeSeqBuilder::new();
    let mut iter = OuterJoin::<A, B, E>::new(a, b);
    while let Some(x) = iter.next() {
        match x? {
            (Some(a), Some(b)) => {
                outer_combine(&a, ab, &b, bb, f, &mut res)?;
            }
            (Some(a), None) => {
                res.push_detached(a, ab)?;
            }
            (None, Some(b)) => {
                res.push_detached(b, bb)?;
            }
            (None, None) => {}
        }
    }
    Ok(res)
}

fn inner_combine<A, B, E, F>(
    a: &TreeNode<A>,
    ab: &A,
    b: &TreeNode<B>,
    bb: &B,
    f: F,
    target: &mut NodeSeqBuilder,
) -> Result<(), E>
where
    A: BlobStore,
    B: BlobStore,
    E: From<A::Error> + From<B::Error> + From<NoError>,
    F: Fn(&TreeValueOptRef<A>, &TreeValueOptRef<B>) -> Result<Option<OwnedTreeValue>, E> + Copy,
{
    todo!()
}

fn inner_combine_children<'a, A, B, E, F>(
    a: NodeSeqIter<'a, A>,
    ab: &A,
    b: NodeSeqIter<'a, B>,
    bb: &B,
    f: F,
) -> Result<NodeSeqBuilder, E>
where
    A: BlobStore,
    B: BlobStore,
    E: From<A::Error> + From<B::Error> + From<NoError>,
    F: Fn(&TreeValueOptRef<A>, &TreeValueOptRef<B>) -> Result<Option<OwnedTreeValue>, E> + Copy,
{
    let mut res = NodeSeqBuilder::new();
    let mut iter = OuterJoin::<A, B, E>::new(a, b);
    while let Some(x) = iter.next() {
        match x? {
            (Some(a), Some(b)) => {
                inner_combine(&a, ab, &b, bb, f, &mut res)?;
            }
            (Some(a), None) => {}
            (None, Some(b)) => {}
            (None, None) => {}
        }
    }
    Ok(res)
}

struct OuterJoin<'a, A: BlobStore, B: BlobStore, E> {
    a: NodeSeqIter<'a, A>,
    b: NodeSeqIter<'a, B>,
    p: PhantomData<E>,
}

impl<'a, A, B, E> OuterJoin<'a, A, B, E>
where
    A: BlobStore,
    B: BlobStore,
    E: From<A::Error> + From<B::Error>,
{
    pub fn new(a: NodeSeqIter<'a, A>, b: NodeSeqIter<'a, B>) -> Self {
        Self {
            a,
            b,
            p: PhantomData,
        }
    }

    // get the next a, assuming we know already that it is not an Err
    #[allow(clippy::type_complexity)]
    fn next_a(&mut self) -> Option<(Option<TreeNode<'_, A>>, Option<TreeNode<'_, B>>)> {
        let a = self.a.next().unwrap();
        Some((Some(a), None))
    }

    // get the next b, assuming we know already that it is not an Err
    #[allow(clippy::type_complexity)]
    fn next_b(&mut self) -> Option<(Option<TreeNode<'_, A>>, Option<TreeNode<'_, B>>)> {
        let b = self.b.next().unwrap();
        Some((None, Some(b)))
    }

    // get the next a and b, assuming we know already that neither is an Err
    #[allow(clippy::type_complexity)]
    fn next_ab(&mut self) -> Option<(Option<TreeNode<'_, A>>, Option<TreeNode<'_, B>>)> {
        let a = self.a.next();
        let b = self.b.next();
        Some((a, b))
    }

    fn next0(&mut self) -> Result<Option<(Option<TreeNode<'_, A>>, Option<TreeNode<'_, B>>)>, E> {
        Ok(
            if let (Some(ak), Some(bk)) = (self.a.peek(), self.b.peek()) {
                match ak.cmp(&bk) {
                    Ordering::Less => self.next_a(),
                    Ordering::Greater => self.next_b(),
                    Ordering::Equal => self.next_ab(),
                }
            } else if let Some(a) = self.a.next() {
                Some((Some(a), None))
            } else if let Some(b) = self.b.next() {
                Some((None, Some(b)))
            } else {
                None
            },
        )
    }

    fn next(&mut self) -> Option<Result<(Option<TreeNode<'_, A>>, Option<TreeNode<'_, B>>), E>> {
        self.next0().transpose()
    }
}

struct Combiner<'a, A: BlobStore, B: BlobStore> {
    a: &'a mut InPlaceNodeSeqBuilder<A>,
    b: NodeSeqIter<'a, B>,
}

impl<'a, A, B> Combiner<'a, A, B>
where
    A: BlobStore,
    B: BlobStore,
    A::Error: From<B::Error>,
{
    pub fn new(a: &'a mut InPlaceNodeSeqBuilder<A>, b: NodeSeqIter<'a, B>) -> Self {
        Self { a, b }
    }

    /// Compare the first element of the lhs builder (a) with the first element of the rhs iterator (b)
    ///
    /// If this methond returns with success, it is guaranteed that neither a nor b produce an error
    fn cmp(&self) -> Result<Option<Ordering>, A::Error> {
        Ok(match (self.a.peek(), self.b.peek()) {
            (Some(a), Some(b)) => Some(a.cmp(&b)),
            (Some(_), None) => Some(Ordering::Less),
            (None, Some(_)) => Some(Ordering::Greater),
            (None, None) => None,
        })
    }
}

fn outer_combine_with<A, B, F>(
    a: InPlaceBuilderRef<A, AtPrefix>,
    ab: &A,
    b: &TreeNode<B>,
    bb: &B,
    f: F,
) -> Result<(), A::Error>
where
    A: BlobStore,
    B: BlobStore,
    A::Error: From<B::Error>,
    F: Fn(&TreeValueRef<A>, &TreeValueRef<B>) -> Result<Option<OwnedTreeValue>, A::Error> + Copy,
{
    let ap = a.peek().load2(ab)?;
    let bp = b.prefix().load2(bb)?;
    let n = common_prefix(ap.as_ref(), bp.as_ref());
    if n == ap.len() && n == bp.len() {
        // prefixes are identical
        // move prefix and value
        let a = a.move_prefix();
        let a = match (a.peek().value_opt(), b.value.value_opt()) {
            (Some(av), Some(bv)) => {
                let r = f(av, bv)?;
                a.push_value_opt(r)
            }
            (Some(_), None) => a.move_value(),
            (None, _) => a.push_converted(b.value, bb)?,
        };
        let bc = b.children().load(bb)?;
        outer_combine_children_with(a, ab, bc.iter(), bb, f)?;
    } else if n == ap.len() {
        // a is a prefix of b
        // value is value of a
        // move prefix and value
        let a = a.move_prefix().move_value();
        let mut bc = NodeSeqBuilder::<B>::new();
        bc.push_shortened(b, bb, n)?;
        outer_combine_children_with(a, &ab, bc.iter(), &bb, f)?;
    } else if n == bp.len() {
        // b is a prefix of a
        // value is value of b
        // split a at n
        let mut child = NodeSeqBuilder::<A>::new();
        // store the last part of the prefix
        child.push_prefix(&ap[n..]);
        // store the first part of the prefix
        // todo: move_prefix_shortened?
        let ap = a.peek().load(ab)?;
        let a = a.push_prefix_ref(&ap[..n]);
        // store value from a in child, if it exists
        child.push_value_ref(a.peek());
        // store the value from b, if it exists
        let a = a.push_converted(b.value(), bb)?;
        // take the children
        child.push_children_ref(a.peek());
        let a = a.set_new_arc(Arc::new(child));
        let bc = b.children().load(bb)?;
        outer_combine_children_with(a, ab, bc.iter(), bb, f)?;
    } else {
        // the two nodes are disjoint
        // value is none
        let mut child = NodeSeqBuilder::<A>::new();
        child.push_prefix(&ap[n..]);
        // todo: move_prefix_shortened?
        let ap = a.peek().load(ab)?;
        let a = a.push_prefix_ref(&ap[..n]);
        child.push_value_ref(a.peek());
        let a = a.push_value_none();
        child.push_children_ref(a.peek());
        let mut children = NodeSeqBuilder::<A>::new();
        // children is just the shortened children a and b in the right order
        if ap[n] <= bp[n] {
            children.push(child.iter().next().unwrap());
            children.push_shortened_converted(b, bb, n)?;
        } else {
            children.push_shortened_converted(b, bb, n)?;
            children.push(child.iter().next().unwrap());
        }
        a.push_new_arc(Arc::new(children));
    }
    Ok(())
}

fn outer_combine_children_with<'a, A, B, F>(
    a: InPlaceBuilderRef<'a, A, AtChildren>,
    ab: &A,
    bc: NodeSeqIter<'a, B>,
    bb: &B,
    f: F,
) -> Result<InPlaceBuilderRef<'a, A, AtPrefix>, A::Error>
where
    A: BlobStore,
    B: BlobStore,
    A::Error: From<B::Error>,
    F: Fn(&TreeValueRef<A>, &TreeValueRef<B>) -> Result<Option<OwnedTreeValue>, A::Error> + Copy,
{
    if bc.is_empty() {
        Ok(a.move_children())
    } else if a.peek().is_empty() {
        let mut res = NodeSeqBuilder::new();
        let mut bc = bc;
        while let Some(b) = bc.next() {
            res.push_converted(b, bb)?;
        }
        Ok(a.push_new_arc(Arc::new(res)))
    } else {
        a.mutate(ab, move |ac| {
            let mut c = Combiner::<A, B>::new(ac, bc);
            while let Some(ordering) = c.cmp()? {
                match ordering {
                    Ordering::Less => {
                        c.a.move_one();
                    }
                    Ordering::Equal => {
                        // the .unwrap() is safe because cmp guarantees that there is a value
                        let b = c.b.next().unwrap();
                        outer_combine_with(c.a.cursor(), ab, &b, bb, f)?;
                    }
                    Ordering::Greater => {
                        // the .unwrap() is safe because cmp guarantees that there is a value
                        let b = c.b.next().unwrap();
                        c.a.insert_converted(b, bb)?;
                    }
                }
            }
            Ok(())
        })
    }
}

enum FindResult<T> {
    // Found an exact match
    Found(T),
    // found a tree for which the path is a prefix, with n remaining chars in the prefix of T
    Prefix {
        // a tree of which the searched path is a prefix
        tree: T,
        // number of remaining elements in the prefix of the tree
        rt: usize,
    },
    // did not find anything, T is the closest match, with n remaining (unmatched) in the prefix of T
    NotFound,
}

/// find a prefix in a tree. Will either return
/// - Found(tree) if we found the tree exactly,
/// - Prefix if we found a tree of which prefix is a prefix
/// - NotFound if there is no tree
fn find<S: BlobStore, T>(
    store: &S,
    tree: &TreeNode<S>,
    prefix: &[u8],
    f: impl Fn(FindResult<&TreeNode<S>>) -> Result<T, S::Error>,
) -> Result<T, S::Error> {
    let tree_prefix = tree.prefix().load(store)?;
    let n = common_prefix(&tree_prefix, prefix);
    // remaining in tree prefix
    let rt = tree_prefix.len() - n;
    // remaining in prefix
    let rp = prefix.len() - n;
    let fr = if rp == 0 && rt == 0 {
        // direct hit
        FindResult::Found(tree)
    } else if rp == 0 {
        // tree is a subtree of prefix
        FindResult::Prefix { tree: tree, rt }
    } else if rt == 0 {
        // prefix is a subtree of tree
        let c = prefix[n];
        let tree_children = tree.children().load(store)?;
        if let Some(child) = tree_children.find(c) {
            return find(store, &child, &prefix[n..], f);
        } else {
            FindResult::NotFound
        }
    } else {
        // disjoint, but we still need to store how far we matched
        FindResult::NotFound
    };
    f(fr)
}

#[derive(Debug, Clone, Default)]
pub struct IterKey(Arc<Vec<u8>>);

impl IterKey {
    fn new(root: &[u8]) -> Self {
        Self(Arc::new(root.to_vec()))
    }

    fn append(&mut self, data: &[u8]) {
        // for typical iterator use, a reference is not kept for a long time, so this will be very cheap
        //
        // in the case a reference is kept, this will make a copy.
        let elems = Arc::make_mut(&mut self.0);
        elems.extend_from_slice(data);
    }

    fn pop(&mut self, n: usize) {
        let elems = Arc::make_mut(&mut self.0);
        elems.truncate(elems.len().saturating_sub(n));
    }
}

impl AsRef<[u8]> for IterKey {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Borrow<[u8]> for IterKey {
    fn borrow(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl core::ops::Deref for IterKey {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

pub struct Iter<S: BlobStore> {
    path: IterKey,
    stack: Vec<(usize, Option<OwnedBlob>, NodeSeqIter2<S>)>,
    store: S,
}

impl<S: BlobStore> Iter<S> {
    fn empty(store: S) -> Self {
        Self {
            stack: Vec::new(),
            path: IterKey::default(),
            store,
        }
    }

    fn new(iter: NodeSeqIter2<S>, store: S, prefix: IterKey) -> Self {
        Self {
            stack: vec![(0, None, iter)],
            path: prefix,
            store,
        }
    }

    fn top_value(&mut self) -> &mut Option<OwnedBlob> {
        &mut self.stack.last_mut().unwrap().1
    }

    fn top_prefix_len(&self) -> usize {
        self.stack.last().unwrap().0
    }

    fn next0(&mut self) -> Result<Option<(IterKey, TreeValueRefWrapper<S>)>, S::Error> {
        while !self.stack.is_empty() {
            if let Some((value, node)) = self.stack.last_mut().unwrap().2.next() {
                let prefix = node.prefix.load2(&self.store)?;
                let prefix_len = prefix.len();
                let children = node.children.load(&self.store)?.owned_iter();
                self.path.append(prefix.as_ref());
                self.stack.push((prefix_len, value, children));
            } else {
                if let Some(value) = self.top_value().take() {
                    let value = TreeValueRefWrapper(value, PhantomData);
                    return Ok(Some((self.path.clone(), value)));
                } else {
                    self.path.pop(self.top_prefix_len());
                    self.stack.pop();
                }
            }
        }
        Ok(None)
    }
}

impl<S: BlobStore> Iterator for Iter<S> {
    type Item = Result<(IterKey, TreeValueRefWrapper<S>), S::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next0() {
            Ok(Some(x)) => Some(Ok(x)),
            Ok(None) => None,
            Err(cause) => {
                // ensure that the next call to next will terminate
                self.stack.clear();
                Some(Err(cause))
            }
        }
    }
}

pub struct Values<S: BlobStore> {
    stack: Vec<NodeSeqIter2<S>>,
    store: S,
}

impl<S: BlobStore> Values<S> {
    fn empty(store: S) -> Self {
        Self {
            stack: Vec::new(),
            store,
        }
    }

    fn new(iter: NodeSeqIter2<S>, store: S) -> Self {
        Self {
            stack: vec![iter],
            store,
        }
    }

    fn next0(&mut self) -> Result<Option<TreeValueRefWrapper<S>>, S::Error> {
        while !self.stack.is_empty() {
            if let Some((value, node)) = self.stack.last_mut().unwrap().next() {
                let children = node.children.load(&self.store)?.owned_iter();
                self.stack.push(children);
                if let Some(value) = value {
                    let value = TreeValueRefWrapper(value, PhantomData);
                    return Ok(Some(value));
                }
            } else {
                self.stack.pop();
            }
        }
        Ok(None)
    }
}

impl<S: BlobStore> Iterator for Values<S> {
    type Item = Result<TreeValueRefWrapper<S>, S::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next0() {
            Ok(Some(x)) => Some(Ok(x)),
            Ok(None) => None,
            Err(cause) => {
                // ensure that the next call to next will terminate
                self.stack.clear();
                Some(Err(cause))
            }
        }
    }
}

// impl<K: Into<OwnedTreePrefix>, V: Into<OwnedTreeValue>> FromIterator<(K, V)> for Tree {
//     fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
//         let mut tree = Tree::empty();
//         for (k, v) in iter.into_iter() {
//             tree.outer_combine_with(
//                 &Tree::single(k.into().as_ref(), v.into().as_ref()),
//                 |_, b| Some(b.to_owned()),
//             );
//         }
//         tree
//     }
// }

impl FromIterator<(Vec<u8>, Vec<u8>)> for Tree {
    fn from_iter<T: IntoIterator<Item = (Vec<u8>, Vec<u8>)>>(iter: T) -> Self {
        let mut tree = Tree::empty();
        for (k, v) in iter.into_iter() {
            tree.outer_combine_with(&Tree::single(k.as_ref(), v.as_ref()), |_, b| {
                Some(b.to_owned())
            });
        }
        tree
    }
}

#[cfg(test)]
mod tests {
    use log::info;
    use proptest::prelude::*;
    use std::{collections::BTreeMap, time::Instant};

    use super::*;

    fn arb_prefix() -> impl Strategy<Value = Vec<u8>> {
        proptest::collection::vec(b'0'..b'9', 0..9)
    }

    fn arb_value() -> impl Strategy<Value = Vec<u8>> {
        proptest::collection::vec(any::<u8>(), 0..9)
    }

    fn arb_tree_contents() -> impl Strategy<Value = BTreeMap<Vec<u8>, Vec<u8>>> {
        proptest::collection::btree_map(arb_prefix(), arb_value(), 0..10)
    }

    fn arb_owned_tree() -> impl Strategy<Value = Tree> {
        arb_tree_contents().prop_map(|x| mk_owned_tree(&x))
    }

    fn mk_owned_tree(v: &BTreeMap<Vec<u8>, Vec<u8>>) -> Tree {
        v.clone().into_iter().collect()
    }

    fn to_btree_map(t: &Tree) -> BTreeMap<Vec<u8>, Vec<u8>> {
        t.iter().map(|(k, v)| (k.to_vec(), v.to_vec())).collect()
    }

    proptest! {

        #[test]
        fn btreemap_tree_roundtrip(x in arb_tree_contents()) {
            let reference = x;
            let tree = mk_owned_tree(&reference);
            let actual = to_btree_map(&tree);
            prop_assert_eq!(reference, actual);
        }

        #[test]
        fn union(a in arb_tree_contents(), b in arb_tree_contents()) {
            let at = mk_owned_tree(&a);
            let bt = mk_owned_tree(&b);
            // check right biased union
            let rbut = at.outer_combine(&bt, |_, b| Some(b.to_owned()));
            let rbu = to_btree_map(&rbut);
            let mut rbu_reference = a.clone();
            for (k, v) in b.clone() {
                rbu_reference.insert(k, v);
            }
            prop_assert_eq!(rbu, rbu_reference);
            // check left biased union
            let lbut = at.outer_combine(&bt, |a, _| Some(a.to_owned()));
            let lbu = to_btree_map(&lbut);
            let mut lbu_reference = b.clone();
            for (k, v) in a.clone() {
                lbu_reference.insert(k, v);
            }
            prop_assert_eq!(lbu, lbu_reference);
        }

        #[test]
        fn union_with(a in arb_owned_tree(), b in arb_owned_tree()) {
            // check right biased union
            // let r1 = a.outer_combine(&b, |_, b| Some(b.to_owned()));
            let mut r2 = a.clone();
            r2.outer_combine_with(&b, |a, b| Some(b.to_owned()));
            // prop_assert_eq!(to_btree_map(&r1), to_btree_map(&r2));
            // check left biased union
            // let r1 = a.outer_combine(&b, |a, _| Some(a.to_owned()));
            let mut r2 = a.clone();
            r2.outer_combine_with(&b, |a, _| Some(a.to_owned()));
            // prop_assert_eq!(to_btree_map(&r1), to_btree_map(&r2));
        }
    }

    #[test]
    fn union_with1() {
        let a = btreemap! { vec![1] => vec![], vec![2] => vec![] };
        let b = btreemap! { vec![1, 2] => vec![], vec![2] => vec![] };
        let mut at = mk_owned_tree(&a);
        let bt = mk_owned_tree(&b);
        at.outer_combine_with(&bt, |a, b| Some(b.to_owned()));
        let rbu = to_btree_map(&at);
        let mut rbu_reference = a.clone();
        for (k, v) in b.clone() {
            rbu_reference.insert(k, v);
        }
        assert_eq!(rbu, rbu_reference);
    }

    #[test]
    fn union_with2() {
        let a = btreemap! { vec![] => vec![] };
        let b = btreemap! { vec![] => vec![0] };
        let mut at = mk_owned_tree(&a);
        let bt = mk_owned_tree(&b);
        at.outer_combine_with(&bt, |a, b| Some(b.to_owned()));
        let rbu = to_btree_map(&at);
        let mut rbu_reference = a.clone();
        for (k, v) in b.clone() {
            rbu_reference.insert(k, v);
        }
        assert_eq!(rbu, rbu_reference);
    }

    #[test]
    fn union_with3() {
        let a = btreemap! { vec![] => vec![] };
        let b = btreemap! { vec![] => vec![], vec![1] => vec![] };
        let mut at = mk_owned_tree(&a);
        let bt = mk_owned_tree(&b);
        at.outer_combine_with(&bt, |a, b| Some(b.to_owned()));
        let rbu = to_btree_map(&at);
        let mut rbu_reference = a.clone();
        for (k, v) in b.clone() {
            rbu_reference.insert(k, v);
        }
        assert_eq!(rbu, rbu_reference);
    }

    #[test]
    fn union_smoke() -> anyhow::Result<()> {
        println!("disjoint");
        let a = Tree::single(b"a".as_ref(), b"1".as_ref());
        let b = Tree::single(b"b".as_ref(), b"2".as_ref());
        let mut r = a;
        r.outer_combine_with(&b, |_, b| Some(b.to_owned()));
        r.dump()?;

        println!("same prefix");
        let a = Tree::single(b"ab".as_ref(), b"1".as_ref());
        let b = Tree::single(b"ab".as_ref(), b"2".as_ref());
        let mut r = a;
        r.outer_combine_with(&b, |_, b| Some(b.to_owned()));
        r.dump()?;

        println!("a prefix of b");
        let a = Tree::single(b"a".as_ref(), b"1".as_ref());
        let b = Tree::single(b"ab".as_ref(), b"2".as_ref());
        let mut r = a;
        r.outer_combine_with(&b, |_, b| Some(b.to_owned()));
        r.dump()?;

        println!("b prefix of a");
        let a = Tree::single(b"ab".as_ref(), b"1".as_ref());
        let b = Tree::single(b"a".as_ref(), b"2".as_ref());
        let mut r = a;
        r.outer_combine_with(&b, |_, b| Some(b.to_owned()));
        r.dump()?;

        Ok(())
    }

    #[test]
    fn build_bench() {
        let elems = (0..2000_000u64)
            .map(|i| {
                if i % 100000 == 0 {
                    println!("{}", i);
                }
                (
                    i.to_string().as_bytes().to_vec(),
                    i.to_string().as_bytes().to_vec(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let elems1 = elems.clone();
        let elems2 = elems.clone();
        let t0 = Instant::now();
        println!("building tree");
        let tree: Tree = elems.into_iter().collect();
        println!("unattached tree {} s", t0.elapsed().as_secs_f64());

        let mut count = 0;
        let t0 = Instant::now();
        for (k, v) in tree.iter() {
            count += 1;
        }
        println!("iterated tree {} s, {}", t0.elapsed().as_secs_f64(), count);

        let mut count = 0;
        let t0 = Instant::now();
        for (k, v) in elems1 {
            count += 1;
        }
        println!(
            "iterated btreemap {} s, {}",
            t0.elapsed().as_secs_f64(),
            count
        );

        let mut count = 0;
        let t0 = Instant::now();
        for v in tree.values() {
            count += 1;
        }
        println!(
            "iterated tree values {} s, {}",
            t0.elapsed().as_secs_f64(),
            count
        );

        let mut count = 0;
        let t0 = Instant::now();
        for v in elems2.values() {
            count += 1;
        }
        println!(
            "iterated btreemap values {} s, {}",
            t0.elapsed().as_secs_f64(),
            count
        );
        // println!("{}", MUTATE_COUNTER.load(std::sync::atomic::Ordering::SeqCst));
        // println!("{}", GROW_COUNTER.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn smoke() {
        let a = Tree::single(b"aaaa", b"b");
        let b = Tree::single(b"aa", b"d");
        let r = a.outer_combine(&b, |_, b| Some(b.to_owned()));
        println!("{:?}", r);
        println!("{:?}", r.node.iter().next().unwrap());

        let mut t = Tree::empty();
        for i in 0u64..100 {
            let txt = i.to_string();
            let b = txt.as_bytes();
            t = t.outer_combine(&Tree::single(b, b), |_, b| Some(b.to_owned()))
        }
        for (k, v) in t.iter() {
            println!("{:?} {:?}", k, v);
        }
        // println!("{:?}", t);
        // println!("{:?}", t.node.iter().next().unwrap().unwrap());
        // t.dump().unwrap();
        // println!("{}", std::mem::size_of::<Arc<String>>());
        // println!("{}", std::mem::size_of::<Option<Arc<String>>>());

        // for i in 0..1000 {
        //     let res = t.try_get(i.to_string().as_bytes()).unwrap();
        //     println!("{:?}", res);
        // }

        println!("---");

        let t0 = std::time::Instant::now();
        let mut t = Tree::empty();
        for i in 0u64..1000000 {
            let txt = i.to_string();
            let b = txt.as_bytes();
            t.outer_combine_with(&Tree::single(b, b), |_, b| Some(b.to_owned()))
        }
        println!("create {}", t0.elapsed().as_secs_f64());

        // for i in 0..10000 {
        //     let res = t.try_get(i.to_string().as_bytes()).unwrap();
        //     println!("{:?}", res);
        // }

        // for (k, v) in t.iter() {
        //     println!("{:?} {:?}", k, v);
        // }

        // println!("{:?}", t);
    }
}
