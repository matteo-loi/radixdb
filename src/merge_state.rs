#![allow(dead_code)]
use crate::{
    iterators::SliceIterator,
    node::TreeNode,
    store::{BlobStore, NoError, NoStore},
    NodeConverter,
};
use binary_merge::{MergeOperation, MergeState};
use core::{fmt, fmt::Debug};
use inplace_vec_builder::InPlaceVecBuilder;
use std::marker::PhantomData;

/// A typical write part for the merge state
pub(crate) trait MergeStateMut: MergeState {
    /// Consume n elements of a
    fn advance_a(&mut self, n: usize, take: bool) -> bool;
    /// Consume n elements of b
    fn advance_b(&mut self, n: usize, take: bool) -> bool;
}

pub(crate) trait MutateInput: MergeStateMut {
    fn source_slices_mut(&mut self) -> (&mut [Self::A], &[Self::B]);
}

/// An in place merge state where the rhs is a reference
pub(crate) struct InPlaceVecMergeStateRef<'a, T: TT> {
    pub a: InPlaceVecBuilder<'a, TreeNode<T::AB>>,
    pub ab: &'a T::AB,
    pub b: SliceIterator<'a, TreeNode<T::BB>>,
    pub bb: &'a T::BB,
    pub c: T::NC,
    pub err: Option<T::E>,
}

impl<'a, T: TT> InPlaceVecMergeStateRef<'a, T> {
    pub fn new(
        a: &'a mut Vec<TreeNode<T::AB>>,
        ab: &'a T::AB,
        b: &'a [TreeNode<T::BB>],
        bb: &'a T::BB,
        c: T::NC,
    ) -> Self {
        Self {
            a: a.into(),
            ab,
            b: SliceIterator(b.as_ref()),
            bb,
            c,
            err: None,
        }
    }
}

impl<'a, T: TT> MergeState for InPlaceVecMergeStateRef<'a, T> {
    type A = TreeNode<T::AB>;
    type B = TreeNode<T::BB>;
    fn a_slice(&self) -> &[TreeNode<T::AB>] {
        self.a.source_slice()
    }
    fn b_slice(&self) -> &[TreeNode<T::BB>] {
        self.b.as_slice()
    }
}

impl<'a, T: TT> MergeStateMut for InPlaceVecMergeStateRef<'a, T> {
    fn advance_a(&mut self, n: usize, take: bool) -> bool {
        self.a.consume(n, take);
        true
    }
    fn advance_b(&mut self, n: usize, take: bool) -> bool {
        if take {
            let iter = &mut self.b;
            // self.a.extend_from_iter((&mut self.b).cloned(), n);
            for _ in 0..n {
                if let Some(node) = iter.next() {
                    match self.c.convert_node(node, self.bb) {
                        Ok(node) => self.a.push(node),
                        Err(err) => {
                            self.err = Some(err.into());
                            return false;
                        }
                    }
                } else {
                    break;
                }
            }
        } else {
            for _ in 0..n {
                let _ = self.b.next();
            }
        }
        true
    }
}

impl<'a, T: TT> MutateInput for InPlaceVecMergeStateRef<'a, T> {
    fn source_slices_mut(&mut self) -> (&mut [Self::A], &[Self::B]) {
        (self.a.source_slice_mut(), self.b.as_slice())
    }
}

impl<'a, T: TT> InPlaceVecMergeStateRef<'a, T> {
    pub fn merge<O: MergeOperation<Self>>(
        a: &'a mut Vec<TreeNode<T::AB>>,
        ab: &'a T::AB,
        b: &'a [TreeNode<T::BB>],
        bb: &'a T::BB,
        c: T::NC,
        o: &O,
    ) -> Result<(), T::E> {
        let mut state = Self::new(a, ab, b, bb, c);
        o.merge(&mut state);
        if let Some(err) = state.err {
            Err(err)
        } else {
            Ok(())
        }
    }
}

/// A merge state where we only track if elements have been produced, and abort as soon as the first element is produced
pub(crate) struct BoolOpMergeState<'a, T: TT> {
    pub a: SliceIterator<'a, TreeNode<T::AB>>,
    pub ab: &'a T::AB,
    pub b: SliceIterator<'a, TreeNode<T::BB>>,
    pub bb: &'a T::BB,
    pub r: bool,
    pub err: Option<T::E>,
}

impl<'a, T: TT> Debug for BoolOpMergeState<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a: {:?}, b: {:?} r: {}",
            self.a_slice(),
            self.b_slice(),
            self.r
        )
    }
}

impl<'a, T: TT> BoolOpMergeState<'a, T> {
    fn new(
        a: &'a [TreeNode<T::AB>],
        ab: &'a T::AB,
        b: &'a [TreeNode<T::BB>],
        bb: &'a T::BB,
    ) -> Self {
        Self {
            a: SliceIterator(a),
            ab,
            b: SliceIterator(b),
            bb,
            err: None,
            r: false,
        }
    }

    pub fn merge<O: MergeOperation<Self>>(
        a: &'a [TreeNode<T::AB>],
        ab: &'a T::AB,
        b: &'a [TreeNode<T::BB>],
        bb: &'a T::BB,
        o: &O,
    ) -> Result<bool, T::E> {
        let mut state = Self::new(a, ab, b, bb);
        o.merge(&mut state);
        if let Some(err) = state.err {
            Err(err)
        } else {
            Ok(state.r)
        }
    }
}

impl<'a, T: TT> MergeState for BoolOpMergeState<'a, T> {
    type A = TreeNode<T::AB>;
    type B = TreeNode<T::BB>;
    fn a_slice(&self) -> &[TreeNode<T::AB>] {
        self.a.as_slice()
    }
    fn b_slice(&self) -> &[TreeNode<T::BB>] {
        self.b.as_slice()
    }
}

impl<'a, T: TT> MergeStateMut for BoolOpMergeState<'a, T> {
    fn advance_a(&mut self, n: usize, take: bool) -> bool {
        if take {
            self.r = true;
            false
        } else {
            self.a.drop_front(n);
            true
        }
    }
    fn advance_b(&mut self, n: usize, take: bool) -> bool {
        if take {
            self.r = true;
            false
        } else {
            self.b.drop_front(n);
            true
        }
    }
}

pub trait TT: Default {
    type AB: BlobStore;
    type BB: BlobStore;
    type E: From<<<Self as TT>::AB as BlobStore>::Error>
        + From<<<Self as TT>::BB as BlobStore>::Error>;
    type NC: NodeConverter<<Self as TT>::BB, <Self as TT>::AB>;
}

pub struct TTI<AB, BB, E, NC>(PhantomData<(AB, BB, E, NC)>);

impl<AB, BB, E, NC> Default for TTI<AB, BB, E, NC> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<
        AB: BlobStore,
        BB: BlobStore,
        E: From<AB::Error> + From<BB::Error>,
        NC: NodeConverter<BB, AB>,
    > TT for TTI<AB, BB, E, NC>
{
    type AB = AB;

    type BB = BB;

    type E = E;

    type NC = NC;
}

impl<AB, BB, E, NC> TTI<AB, BB, E, NC> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[derive(Default)]
pub struct NoStoreT;

impl TT for NoStoreT {
    type AB = NoStore;

    type BB = NoStore;

    type E = NoError;

    type NC = NoConverter;
}

#[derive(Clone, Copy)]
pub struct NoConverter;

impl NodeConverter<NoStore, NoStore> for NoConverter {}

#[derive(Clone, Copy)]
pub struct DetachConverter;

impl<T: BlobStore> NodeConverter<T, NoStore> for DetachConverter {}

#[derive(Clone, Copy)]
pub(crate) struct DummyConverter;

impl<A: BlobStore, B: BlobStore> NodeConverter<A, B> for DummyConverter {
    fn convert_node(
        &self,
        node: &TreeNode<A>,
        store: &A,
    ) -> Result<TreeNode<B>, <A as BlobStore>::Error> {
        todo!()
    }

    fn convert_value(
        &self,
        value: &crate::node::TreeValue<A>,
        store: &A,
    ) -> Result<crate::node::TreeValue<B>, <A as BlobStore>::Error> {
        todo!()
    }
}

/// A merge state where we build into a new vec
pub(crate) struct VecMergeState<'a, T: TT> {
    pub a: SliceIterator<'a, TreeNode<T::AB>>,
    pub ab: &'a T::AB,
    pub b: SliceIterator<'a, TreeNode<T::BB>>,
    pub bb: &'a T::BB,
    pub r: Vec<TreeNode>,
    pub err: Option<T::E>,
}

impl<'a, T: TT> Debug for VecMergeState<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a: {:?}, b: {:?}", self.a_slice(), self.b_slice(),)
    }
}

impl<'a, T: TT> VecMergeState<'a, T> {
    fn new(
        a: &'a [TreeNode<T::AB>],
        ab: &'a T::AB,
        b: &'a [TreeNode<T::BB>],
        bb: &'a T::BB,
        r: Vec<TreeNode>,
    ) -> Self {
        Self {
            a: SliceIterator(a),
            ab,
            b: SliceIterator(b),
            bb,
            r,
            err: None,
        }
    }

    fn into_vec(self) -> std::result::Result<Vec<TreeNode>, T::E> {
        if let Some(err) = self.err {
            Err(err)
        } else {
            Ok(self.r)
        }
    }

    pub fn merge<O: MergeOperation<Self>>(
        a: &'a [TreeNode<T::AB>],
        ab: &'a T::AB,
        b: &'a [TreeNode<T::BB>],
        bb: &'a T::BB,
        o: &'a O,
    ) -> std::result::Result<Vec<TreeNode>, T::E> {
        let t: Vec<TreeNode> = Vec::new();
        let mut state = Self::new(a, ab, b, bb, t);
        o.merge(&mut state);
        state.into_vec()
    }
}

impl<'a, T: TT> MergeState for VecMergeState<'a, T> {
    type A = TreeNode<T::AB>;
    type B = TreeNode<T::BB>;
    fn a_slice(&self) -> &[TreeNode<T::AB>] {
        self.a.as_slice()
    }
    fn b_slice(&self) -> &[TreeNode<T::BB>] {
        self.b.as_slice()
    }
}

impl<'a, T: TT> MergeStateMut for VecMergeState<'a, T> {
    fn advance_a(&mut self, n: usize, take: bool) -> bool {
        if take {
            self.r.reserve(n);
            for e in self.a.take_front(n).iter() {
                match e.detached(self.ab) {
                    Ok(e) => self.r.push(e),
                    Err(cause) => {
                        self.err = Some(cause.into());
                        return false;
                    }
                }
            }
        } else {
            self.a.drop_front(n);
        }
        true
    }
    fn advance_b(&mut self, n: usize, take: bool) -> bool {
        if take {
            self.r.reserve(n);
            for e in self.b.take_front(n).iter() {
                match e.detached(self.bb) {
                    Ok(e) => self.r.push(e),
                    Err(cause) => {
                        self.err = Some(cause.into());
                        return false;
                    }
                }
            }
        } else {
            self.b.drop_front(n);
        }
        true
    }
}
