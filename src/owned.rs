use anyhow::Context;
use binary_merge::MergeOperation;
use std::{fmt::Debug, marker::PhantomData, sync::Arc, collections::BTreeMap, cmp::Ordering};

use crate::merge_state::{VecMergeState, InPlaceVecMergeStateRef, MutateInput, MergeStateMut};

use super::*;

#[repr(C)]
pub struct FlexRef<T>([u8; 8], PhantomData<T>);

impl<T> Debug for FlexRef<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(id) = self.id_u64() {
            f.debug_tuple("FlexRef::Id").field(&id).finish()
        } else if let Some(data) = self.inline_as_ref() {
            f.debug_tuple("FlexRef::Inline").field(&Hex(data)).finish()
        } else if let Some(arc) = self.owned_arc_ref() {
            f.debug_struct("FlexRef::Owned")
                .field("p", &Arc::as_ptr(arc))
                .field("c", &Arc::strong_count(arc))
                .finish()
        } else {
            f.debug_tuple("FlexRef::None").finish()
        }
    }
}

impl<T> FlexRef<T> {
    /// the none marker value
    const fn none() -> Self {
        Self(NONE_ARRAY, PhantomData)
    }

    /// try to create an id from an u64, if it fits
    fn id_from_u64(value: u64) -> Option<Self> {
        let mut bytes = value.to_be_bytes();
        if bytes[0] == 0 || bytes[1] == 0 {
            bytes.rotate_left(1);
            bytes[0] = ID_MASK[0];
            bytes[7] = ID_MASK[7];
            Some(Self(bytes, PhantomData))
        } else {
            None
        }
    }

    /// the inline empty array
    const fn inline_empty_array() -> Self {
        Self(EMPTY_INLINE_ARRAY, PhantomData)
    }

    /// try to create an inline value from a slice, if it fits
    fn inline_from_slice(value: &[u8]) -> Option<Self> {
        let len = value.len();
        if len < 7 {
            let mut res = [0u8; 8];
            res[1..=len].copy_from_slice(value);
            let marker = ((len as u8) << 1) | 1;
            res[0] = marker;
            res[7] = marker;
            Some(Self(res, PhantomData))
        } else {
            None
        }
    }

    /// create an owned from an arc to a sized thing
    fn owned_from_arc(arc: Arc<T>) -> Self {
        let addr: usize = unsafe { std::mem::transmute(arc) };
        Self(from_ptr(addr), PhantomData)
    }

    fn inline_as_ref(&self) -> Option<&[u8]> {
        if self.is_inline() {
            let len = (self.0[0] >> 1) as usize;
            Some(&self.0[1..=len])
        } else {
            None
        }
    }

    fn id_u64(&self) -> Option<u64> {
        if self.is_id() {
            let mut res = [0u8; 8];
            res[2..].copy_from_slice(&self.0[1..7]);
            Some(u64::from_be_bytes(res))
        } else {
            None
        }
    }

    fn is_arc(&self) -> bool {
        is_pointer(self.0)
    }

    fn is_copy(&self) -> bool {
        // for now, the only thing that is not copy is an arc
        !self.is_arc()
    }

    fn is_inline(&self) -> bool {
        !self.is_arc() && !self.is_none() && !self.is_id()
    }

    fn is_id(&self) -> bool {
        self.0[0] == ID_MASK[0] && self.0[7] == ID_MASK[7]
    }

    fn is_none(&self) -> bool {
        self.0 == NONE_ARRAY
    }

    fn owned_take_arc(&mut self) -> Option<Arc<T>> {
        if is_pointer(self.0) {
            let res = arc(self.0);
            self.0 = NONE_ARRAY;
            Some(res)
        } else {
            None
        }
    }

    fn into_arc(self) -> Option<Arc<T>> {
        if is_pointer(self.0) {
            let res = arc(self.0);
            std::mem::forget(self);
            Some(res)
        } else {
            None
        }
    }

    fn owned_arc_ref(&self) -> Option<&Arc<T>> {
        if is_pointer(self.0) {
            Some(arc_ref(&self.0))
        } else {
            None
        }
    }
}

fn slice_cast<T, U>(src: &[T]) -> anyhow::Result<&[U]> {
    let (ptr, tsize): (usize, usize) = unsafe { std::mem::transmute(src) };
    let bytes = tsize * std::mem::size_of::<T>();
    anyhow::ensure!(ptr % std::mem::align_of::<U>() == 0, "pointer is not properly aligned for target type");    
    anyhow::ensure!(bytes % std::mem::size_of::<U>() == 0, "byte size is not a multiple of target size");
    let usize = bytes / std::mem::size_of::<U>();
    Ok(unsafe { std::mem::transmute((ptr, usize)) })
}

impl FlexRef<Vec<u8>> {
    fn inline_or_owned_from_slice(value: &[u8]) -> Self {
        if let Some(res) = FlexRef::inline_from_slice(value) {
            res
        } else {
            FlexRef::owned_from_arc(Arc::new(value.to_vec()))
        }
    }
}

impl<T> Clone for FlexRef<T> {
    fn clone(&self) -> Self {
        if let Some(arc) = self.owned_arc_ref() {
            Self::owned_from_arc(arc.clone())
        } else {
            Self(self.0, PhantomData)
        }
    }
}

impl<T> Drop for FlexRef<T> {
    fn drop(&mut self) {
        if let Some(arc) = self.owned_take_arc() {
            drop(arc)
        }
    }
}

pub trait BlobStore: Debug {
    fn bytes(&self, id: u64) -> anyhow::Result<&[u8]>;

    fn append(&mut self, data: &[u8]) -> anyhow::Result<u64>;
}

#[repr(transparent)]
#[derive(Clone)]
pub struct Inline(FlexRef<()>);

#[repr(transparent)]
#[derive(Clone)]
pub struct TreeValue(FlexRef<Vec<u8>>);

/// A tree prefix is an optional blob, that is stored either inline, on the heap, or as an id
impl TreeValue {
    fn none() -> Self {
        Self(FlexRef::none())
    }

    fn is_none(&self) -> bool {
        self.0.is_none()
    }

    fn from_slice(data: &[u8]) -> Self {
        Self(FlexRef::inline_or_owned_from_slice(data))
    }

    fn load<'a>(&'a self, store: &'a Box<dyn BlobStore>) -> anyhow::Result<Option<&[u8]>> {
        if let Some(data) = self.0.inline_as_ref() {
            Ok(Some(data))
        } else if let Some(arc) = self.0.owned_arc_ref() {
            Ok(Some(arc.as_ref()))
        } else if let Some(id) = self.0.id_u64() {
            store.bytes(id).map(Some)
        } else if self.0.is_none() {
            Ok(None)
        } else {
            unreachable!("invalid state of a TreePrefix");
        }
    }

    fn validate_serialized(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.0.is_inline() || self.0.is_id()|| self.0.is_none());
        Ok(())
    }

    /// detaches the value from the store. on success it will either be none, inline, or owned
    fn detach(&mut self, store: &Box<dyn BlobStore>) -> anyhow::Result<()> {
        if let Some(id) = self.0.id_u64() {
            let slice = store.bytes(id)?;
            self.0 = FlexRef::inline_or_owned_from_slice(slice);
        }
        Ok(())
    }

    fn detached(&self, store: &Box<dyn BlobStore>) -> anyhow::Result<Self> {
        let mut t = self.clone();
        t.detach(store)?;
        Ok(t)
    }

    /// attaches the value to the store. on success it will either be none, inline or id
    ///
    /// if the value is already attached, it is assumed that it is to the store, so it is a noop
    fn attach(&mut self, store: &mut Box<dyn BlobStore>) -> anyhow::Result<()> {
        if let Some(arc) = self.0.owned_arc_ref() {
            let id = store.append(arc.as_ref())?;
            self.0 = FlexRef::id_from_u64(id).context("id too large")?;
        }
        Ok(())
    }
}

impl Debug for TreeValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(data) = self.0.inline_as_ref() {
            f.debug_tuple("Inline")
                .field(&Hex(data))
                .finish()
        } else if let Some(arc) = self.0.owned_arc_ref() {
            let data = arc.as_ref();
            if data.len() <= 32 {
                f.debug_tuple("Owned").field(&Hex(data)).finish()
            } else {
                write!(
                    f,
                    "Owned({}...,{} bytes)",
                    &Hex(&data[0..32]),
                    data.len()
                )
            }
        } else if let Some(id) = self.0.id_u64() {
            f.debug_tuple("Id").field(&id).finish()
        } else if self.0.is_none() {
            f.debug_tuple("None").finish()
        } else {
            f.debug_tuple("Invalid").finish()
        }
    }
}

impl Default for TreeValue {
    fn default() -> Self {
        Self::none()
    }
}

/// A tree prefix is a blob, that is stored either inline, on the heap, or as an id
#[repr(transparent)]
#[derive(Clone)]
pub struct TreePrefix(FlexRef<Vec<u8>>);

impl TreePrefix {
    fn validate_serialized(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.0.is_inline() || self.0.is_id());
        Ok(())
    }

    fn from_slice(data: &[u8]) -> Self {
        Self(FlexRef::inline_or_owned_from_slice(data))
    }

    fn load<'a>(&'a self, store: &'a Box<dyn BlobStore>) -> anyhow::Result<&[u8]> {
        if let Some(data) = self.0.inline_as_ref() {
            Ok(data)
        } else if let Some(arc) = self.0.owned_arc_ref() {
            Ok(arc.as_ref())
        } else if let Some(id) = self.0.id_u64() {
            store.bytes(id)
        } else {
            unreachable!("invalid state of a TreePrefix");
        }
    }

    /// detaches the prefix from the store. on success it will either be inline or owned
    fn detach(&mut self, store: &Box<dyn BlobStore>) -> anyhow::Result<()> {
        if let Some(id) = self.0.id_u64() {
            let slice = store.bytes(id)?;
            self.0 = FlexRef::inline_or_owned_from_slice(slice);
        }
        Ok(())
    }

    /// attaches the prefix to the store. on success it will either be inline or id
    ///
    /// if the prefix is already attached, it is assumed that it is to the store, so it is a noop
    fn attach(&mut self, store: &mut Box<dyn BlobStore>) -> anyhow::Result<()> {
        if let Some(arc) = self.0.owned_arc_ref() {
            let id = store.append(arc.as_ref())?;
            self.0 = FlexRef::id_from_u64(id).context("id too large")?;
        }
        Ok(())
    }
}

impl Debug for TreePrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(data) = self.0.inline_as_ref() {
            f.debug_tuple("Inline")
                .field(&Hex(data))
                .finish()
        } else if let Some(arc) = self.0.owned_arc_ref() {
            let data = arc.as_ref();
            if data.len() <= 32 {
                f.debug_tuple("Owned")
                    .field(&Hex(data))
                    .finish()
            } else {
                write!(
                    f,
                    "Owned({}...,{} bytes)",
                    &Hex(&data[0..32]),
                    data.len()
                )
            }
        } else if let Some(id) = self.0.id_u64() {
            f.debug_tuple("Id").field(&id).finish()
        } else {
            f.debug_tuple("Invalid").finish()
        }
    }
}

impl Default for TreePrefix {
    fn default() -> Self {
        Self(FlexRef::inline_empty_array())
    }
}

/// Tree children are an optional array, that are stored either on the heap, or as an id
#[repr(transparent)]
#[derive(Clone)]
pub struct TreeChildren(FlexRef<Vec<TreeNode>>);

impl TreeChildren {
    fn from_arc(data: Arc<Vec<TreeNode>>) -> Self {
        Self(FlexRef::owned_from_arc(data))
    }

    fn from_vec(vec: Vec<TreeNode>) -> Self {
        if vec.is_empty() {
            Self::default()
        } else {
            Self::from_arc(Arc::new(vec))
        }
    }

    fn from_slice(slice: &[TreeNode]) -> Self {
        if slice.is_empty() {
            Self::default()
        } else {
            Self::from_vec(slice.to_vec())
        }
    }

    fn empty() -> Self {
        Self(FlexRef::none())
    }

    fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    fn load<'a>(&'a self, store: &'a Box<dyn BlobStore>) -> anyhow::Result<&[TreeNode]> {
        if let Some(arc) = self.0.owned_arc_ref() {
            Ok(arc.as_ref().as_ref())
        } else if let Some(id) = self.0.id_u64() {
            let bytes = store.bytes(id)?;
            Ok(TreeNode::nodes_from_bytes(bytes)?)
        } else if self.0.is_none() {
            Ok(&[])
        } else {
            unreachable!("invalid state of a TreePrefix");
        }
    }

    fn validate_serialized(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.0.is_id()|| self.0.is_none());
        Ok(())
    }

    fn detach(&mut self, store: &Box<dyn BlobStore>, recursive: bool) -> anyhow::Result<()> {
        if let Some(id) = self.0.id_u64() {
            let mut children = TreeNode::nodes_from_bytes(store.bytes(id)?)?.to_vec();
            if recursive {
                for child in &mut children {
                    child.detach(store, recursive)?;
                }
            }
            self.0 = FlexRef::owned_from_arc(Arc::new(children))
        }
        Ok(())
    }

    /// attaches the children to the store. on success it be an id
    fn attach(&mut self, store: &mut Box<dyn BlobStore>) -> anyhow::Result<()> {
        if let Some(arc) = self.0.owned_arc_ref() {
            let mut children = arc.as_ref().clone();
            for child in &mut children {
                child.attach(store)?;
            }
            let bytes = TreeNode::slice_to_bytes(&children)?;
            self.0 = FlexRef::id_from_u64(store.append(&bytes)?).context("id too large")?;
        }
        Ok(())
    }

    fn detached(&self, store: &Box<dyn BlobStore>, recursive: bool) -> anyhow::Result<Self> {
        let mut t = self.clone();
        t.detach(store, recursive)?;
        Ok(t)
    }
}

impl Debug for TreeChildren {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(arc) = self.0.owned_arc_ref() {
            let data = arc.as_ref();
            write!(f, "Owned(len={})", data.len())
        } else if let Some(id) = self.0.id_u64() {
            f.debug_tuple("Id").field(&id).finish()
        } else if self.0.is_none() {
            f.debug_tuple("None").finish()
        } else {
            f.debug_tuple("Invalid").finish()
        }
    }
}

impl Default for TreeChildren {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Default, Clone, Debug)]
#[repr(C)]
pub struct TreeNode {
    prefix: TreePrefix,
    value: TreeValue,
    children: TreeChildren,
}

impl TreeNode {

    fn validate_serialized(&self) -> anyhow::Result<()> {
        self.prefix.validate_serialized()?;
        self.value.validate_serialized()?;
        self.children.validate_serialized()?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty() && self.value.is_none()
    }

    pub fn nodes_from_bytes(bytes: &[u8]) -> anyhow::Result<&[TreeNode]> {
        let res: &[TreeNode] = slice_cast(bytes)?;
        for node in res {
            node.validate_serialized()?;
        }
        Ok(res)
    }

    pub fn slice_to_bytes(nodes: &[Self]) -> anyhow::Result<Vec<u8>> {
        let mut res = Vec::with_capacity(nodes.len() * 24);
        for node in nodes {
            anyhow::ensure!(!node.prefix.0.is_arc());
            anyhow::ensure!(!node.value.0.is_arc());
            anyhow::ensure!(!node.children.0.is_arc());
            res.extend_from_slice(&node.prefix.0 .0);
            res.extend_from_slice(&node.value.0 .0);
            res.extend_from_slice(&node.children.0 .0);
        }
        Ok(res)
    }

    pub fn leaf(value: &[u8]) -> Self {
        Self {
            value: TreeValue::from_slice(value),
            ..Default::default()
        }
    }

    pub fn single(key: &[u8], value: &[u8]) -> Self {
        Self {
            prefix: TreePrefix::from_slice(key),
            value: TreeValue::from_slice(value),
            ..Default::default()
        }
    }

    pub fn new(key: &[u8], value: Option<&[u8]>, children: &[Self]) -> Self {
        Self {
            prefix: TreePrefix::from_slice(key),
            value: value.map(TreeValue::from_slice).unwrap_or_default(),
            children: TreeChildren::from_vec(children.to_vec()),
        }
    }

    /// detach the node from its `store`.
    ///
    /// if `recursive` is true, the tree gets completely detached, otherwise just one level deep.
    pub fn detach(&mut self, store: &Box<dyn BlobStore>, recursive: bool) -> anyhow::Result<()> {
        self.prefix.detach(store)?;
        self.value.detach(store)?;
        self.children.detach(store, recursive)?;
        Ok(())
    }

    /// attaches the node components to the store
    pub fn attach(&mut self, store: &mut Box<dyn BlobStore>) -> anyhow::Result<()> {
        self.prefix.attach(store)?;
        self.value.attach(store)?;
        self.children.attach(store)?;
        Ok(())
    }

    pub fn detached_shortened(&self, store: &Box<dyn BlobStore>, n: usize, recursive: bool) -> anyhow::Result<Self> {
        todo!()
    }

    pub fn unsplit(&mut self) {
        todo!()
    }
}

#[derive(Default, Clone)]
struct MemStore {
    data: BTreeMap<u64, Arc<Vec<u8>>>
}

impl Debug for MemStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut builder = f.debug_map();
        for (id, v) in &self.data {
            builder.entry(&id, &Hex(v.as_ref()));
        }
        builder.finish()
    }
}
impl BlobStore for MemStore {
    fn bytes(&self, id: u64) -> anyhow::Result<&[u8]> {
        self.data.get(&id).map(|x| x.as_ref().as_ref()).context("value not found")
    }

    fn append(&mut self, data: &[u8]) -> anyhow::Result<u64> {
        let max = self.data.keys().next_back().cloned().unwrap_or(0);
        let id = max + 1;
        let data = Arc::new(data.to_vec());
        self.data.insert(id, data);
        Ok(id)
    }
}

/// Outer combine two trees with a function f
fn outer_combine(
    a: &TreeNode,
    ab: &Box<dyn BlobStore>,
    b: &TreeNode,
    bb: &Box<dyn BlobStore>,
    f: impl Fn(TreeValue, TreeValue) -> TreeValue + Copy,
) -> anyhow::Result<TreeNode> {
    let ap = a.prefix.load(ab)?;
    let bp = b.prefix.load(bb)?;
    let n = common_prefix(ap, bp);
    let prefix = TreePrefix::from_slice(&ap[..n]);
    let mut children;
    let value;
    let av = || a.value.detached(ab);
    let bv = || b.value.detached(bb);
    if n == ap.len() && n == bp.len() {
        // prefixes are identical
        value = if a.value.0.is_none() {
            if b.value.0.is_none() {
                // both none - none
                TreeValue::default()
            } else {
                // detach and take b
                bv()?
            }
        } else {
            if b.value.0.is_none() {
                // detach and take a
                av()?
            } else {
                // call the combine fn
                f(av()?, bv()?)
            }
        };
        children = VecMergeState::merge(
            &a.children.load(ab)?,
            ab,
            &b.children.load(bb)?,
            bb,
            OuterCombineOp(f),
        );
    } else if n == ap.len() {
        // a is a prefix of b
        // value is value of a
        value = av()?;
        let b = b.detached_shortened(bb, n, true)?;
        children = VecMergeState::merge(
            &a.children.load(ab)?,
            ab,
            &b.children.load(bb)?,
            bb,
            OuterCombineOp(f),
        );
    } else if n == bp.len() {
        // b is a prefix of a
        // value is value of b
        value = bv()?;
        let a = a.detached_shortened(ab, n, true)?;
        children = VecMergeState::merge(
            &a.children.load(ab)?,
            ab,
            &b.children.load(bb)?,
            bb,
            OuterCombineOp(f),
        );
    } else {
        // the two nodes are disjoint
        // value is none
        value = TreeValue::default();
        // children is just the shortened children a and b in the right order
        let mut a = a.detached_shortened(ab, n, true)?;
        let mut b = b.detached_shortened(bb, n, true)?;
        if ap[n] > bp[n] {
            std::mem::swap(&mut a, &mut b);
        }
        children = Vec::with_capacity(2);
        children.push(a);
        children.push(b);
    }    
    let mut res = TreeNode {
        prefix,
        value,
        children: TreeChildren::from_vec(children),
    };
    res.unsplit();
    Ok(res)
}

/// In place merge operation
struct OuterCombineOp<F>(F);

// impl<'a, F> MergeOperation<InPlaceVecMergeStateRef<'a, TreeNode>>
//     for OuterCombineOp<F>
// where
//     F: Fn(TreeValue, TreeValue) -> TreeValue + Copy,
// {
//     fn cmp(&self, a: &TreeNode, b: &TreeNode) -> Ordering {
//         a.prefix()[0].cmp(&b.prefix()[0])
//     }
//     fn from_a(&self, m: &mut InPlaceVecMergeStateRef<'a, TreeNode>, n: usize) -> bool {
//         m.advance_a(n, true)
//     }
//     fn from_b(&self, m: &mut InPlaceVecMergeStateRef<'a, TreeNode>, n: usize) -> bool {
//         m.advance_b(n, true)
//     }
//     fn collision(&self, m: &mut InPlaceVecMergeStateRef<'a, TreeNode>) -> bool {
//         let (a, b) = m.source_slices_mut();
//         let av = &mut a[0];
//         let bv = &b[0];
//         av.outer_combine_with(bv, self.0);
//         // we have modified av in place. We are only going to take it over if it
//         // is non-empty, otherwise we skip it.
//         let take = !av.is_empty();
//         m.advance_a(1, take) && m.advance_b(1, false)
//     }
// }

impl<'a, F>
    MergeOperation<VecMergeState<'a>> for OuterCombineOp<F>
where
    F: Fn(TreeValue, TreeValue) -> TreeValue + Copy,
{
    fn cmp(&self, a: &TreeNode, b: &TreeNode) -> Ordering {
        todo!()
        // a.prefix()[0].cmp(&b.prefix()[0])
    }
    fn from_a(
        &self,
        m: &mut VecMergeState<'a>,
        n: usize,
    ) -> bool {
        m.advance_a(n, true)
    }
    fn from_b(
        &self,
        m: &mut VecMergeState<'a>,
        n: usize,
    ) -> bool {
        m.advance_b(n, true)
    }
    fn collision(
        &self,
        m: &mut VecMergeState<'a>,
    ) -> bool {
        let a = m.a.next().unwrap();
        let b = m.b.next().unwrap();
        match outer_combine(a, m.ab, b, m.bb, self.0) {
            Ok(res) => {
                if !res.is_empty() {
                    m.r.push(res);
                }
                true
            },
            Err(cause) => {
                m.err = Some(cause);
                false
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use std::{sync::Arc};

    use crate::owned::{TreeNode, MemStore, BlobStore};

    use super::FlexRef;

    #[test]
    fn flex_ref_debug() {
        let x = FlexRef::<()>::none();
        println!("{:?}", x);
        let x = FlexRef::<()>::inline_from_slice(b"abcd");
        println!("{:?}", x);
        let x = FlexRef::<()>::id_from_u64(1234);
        println!("{:?}", x);
        let arc = Arc::new(b"abcd".to_vec());
        let a1 = arc.clone();
        let a2 = arc.clone();
        // base addr of the arc
        let u1: usize = unsafe { std::mem::transmute(a1) };
        // content addr of the arc, 16 bytes larger on 64 bit
        let u2 = Arc::into_raw(a2) as usize;
        // println!("{} {} {}", u1, u2, u2 - u1);
        let x = FlexRef::owned_from_arc(arc);
        println!("{:?}", x);
    }

    #[test]
    fn tree_node_create() -> anyhow::Result<()> {
        let store: Box<dyn BlobStore> = Box::new(MemStore::default());
        let node = TreeNode::leaf(b"abcd");
        assert!(node.prefix.load(&store)? == &[]);
        assert!(node.value.load(&store)? == Some(b"abcd"));
        assert!(node.children.load(&store)?.is_empty());
        println!("{:?}", node);
        let node = TreeNode::leaf(b"abcdefgh");
        println!("{:?}", node);
        let node = TreeNode::single(b"abcd", b"ijklmnop");
        println!("{:?}", node);
        let node = TreeNode::single(b"abcdefgh", b"ijklmnop");
        println!("{:?}", node);
        let node = TreeNode::new(b"a", None, &[node]);
        println!("{:?}", node);        
        Ok(())
    }

    #[test]
    fn tree_node_attach_detach() -> anyhow::Result<()> {
        let mut store: Box<dyn BlobStore> = Box::new(MemStore::default());
        let node = TreeNode::single(b"abcdefgh", b"ijklmnop");
        let mut node = TreeNode::new(b"a", None, &[node]);
        println!("{:?}", node);
        node.attach(&mut store)?;
        println!("{:?}", node);
        node.detach(&mut store, true)?;
        println!("{:?}", node);
        println!("{:?}", store);
        Ok(())
    }
}
