use crate::collections::vec::SVec;
use crate::primitive::StackAllocated;
use speedy::{Context, LittleEndian, Readable, Reader, Writable, Writer};
use std::mem::size_of;

const B: usize = 6;
const CAPACITY: usize = 2 * B - 1;
const MIN_LEN_AFTER_SPLIT: usize = B - 1;

pub struct SBTreeMap<K, V, AK, AV> {
    root: BTreeNode<K, V, AK, AV>,
    len: u64,
}

impl<AK, AV, K, V> SBTreeMap<K, V, AK, AV> {
    pub fn new() -> Self {
        Self {
            root: BTreeNode::default(),
            len: 0,
        }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<AK, AV, K: Ord + StackAllocated<K, AK>, V: StackAllocated<V, AV>> SBTreeMap<K, V, AK, AV>
where
    [(); size_of::<BTreeNode<K, V, AK, AV>>()]: Sized,
    [(); size_of::<AK>()]: Sized,
    [(); size_of::<AV>()]: Sized,
{
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let res = if self.root.keys.len() == CAPACITY {
            let mut temp = BTreeNode::new(false, false);

            self.root.is_root = false;
            temp.is_root = true;
            let old_root = std::mem::replace(&mut self.root, temp);

            self.root.get_children_mut().insert(0, old_root);

            Self::split_child(&mut self.root, 0);
            let res = Self::insert_non_full(&mut self.root, key, value);

            res
        } else {
            Self::insert_non_full(&mut self.root, key, value)
        };

        if res.is_none() {
            self.len += 1;
        }

        res
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let res = Self::_delete(&mut self.root, key)?;
        self.len -= 1;

        Some(res)
    }

    pub unsafe fn drop(mut self) {
        let children = self.root.get_children();

        while let Some(child_node) = self.root.get_children_mut().pop() {
            Self::_drop(child_node);
        }
    }

    unsafe fn _drop(node: BTreeNode<K, V, AK, AV>) {
        for i in 0..node.get_children().len() {
            Self::_drop(node.get_children().get_copy(i).unwrap());
        }

        node.drop();
    }

    pub fn get_copy(&self, key: &K) -> Option<V> {
        Self::_get(&self.root, key)
    }

    fn _get(node: &BTreeNode<K, V, AK, AV>, key: &K) -> Option<V> {
        match node.keys.binary_search_by(|k| k.cmp(key)) {
            Ok(idx) => node.values.get_copy(idx),
            Err(idx) => {
                let child = node.get_children().get_copy(idx)?;
                Self::_get(&child, key)
            }
        }
    }

    pub fn contains_key(&self, key: &K) -> bool {
        Self::_contains_key(&self.root, key)
    }

    fn _contains_key(node: &BTreeNode<K, V, AK, AV>, key: &K) -> bool {
        match node.keys.binary_search_by(|k| k.cmp(key)) {
            Ok(_) => true,
            Err(idx) => {
                if let Some(child) = node.get_children().get_copy(idx) {
                    Self::_contains_key(&child, key)
                } else {
                    false
                }
            }
        }
    }

    fn insert_non_full(node: &mut BTreeNode<K, V, AK, AV>, key: K, value: V) -> Option<V> {
        match node.keys.binary_search_by(|k| k.cmp(&key)) {
            Ok(idx) => Some(node.values.replace(idx, value)),
            Err(mut idx) => {
                if node.is_leaf {
                    node.keys.insert(idx, key);
                    node.values.insert(idx, value);

                    None
                } else {
                    if node.get_children().get_copy(idx).unwrap().keys.len() == CAPACITY {
                        Self::split_child(node, idx);

                        if key.gt(&node.keys.get_copy(idx).unwrap()) {
                            idx += 1;
                        }
                    }

                    let mut child = node.get_children().get_copy(idx).unwrap();
                    let result = Self::insert_non_full(&mut child, key, value);

                    node.get_children_mut().replace(idx, child);

                    result
                }
            }
        }
    }

    fn split_child(node: &mut BTreeNode<K, V, AK, AV>, idx: usize) {
        let mut child = node.get_children().get_copy(idx).unwrap();
        let mut new_child = BTreeNode::<K, V, AK, AV>::new(child.is_leaf, false);

        for _ in 0..MIN_LEN_AFTER_SPLIT {
            new_child.keys.push(child.keys.remove(B));
            new_child.values.push(child.values.remove(B));
        }
        node.keys
            .insert(idx, child.keys.remove(MIN_LEN_AFTER_SPLIT));
        node.values
            .insert(idx, child.values.remove(MIN_LEN_AFTER_SPLIT));

        if !child.is_leaf {
            for _ in 0..B {
                let grand_child = child.get_children_mut().remove(B);

                new_child.get_children_mut().push(grand_child);
            }
        }

        node.get_children_mut().replace(idx, child);
        node.get_children_mut().insert(idx + 1, new_child);
    }

    fn _delete(node: &mut BTreeNode<K, V, AK, AV>, key: &K) -> Option<V> {
        match node.keys.binary_search_by(|k| k.cmp(key)) {
            Ok(idx) => {
                if node.is_leaf {
                    node.keys.remove(idx);
                    let v = node.values.remove(idx);

                    Some(v)
                } else {
                    Self::delete_internal_node(node, key, idx)
                }
            }
            Err(idx) => {
                let mut merged = false;

                if node.is_leaf {
                    return None;
                }

                let mut child = node.get_children().get_copy(idx).unwrap();

                if child.keys.len() >= B {
                    let res = Self::_delete(&mut child, key);
                    node.get_children_mut().replace(idx, child);

                    res
                } else {
                    if idx != 0 && idx + 1 < node.get_children().len() {
                        let left_child_sibling = node.get_children().get_copy(idx - 1).unwrap();
                        let right_child_sibling = node.get_children().get_copy(idx + 1).unwrap();

                        if left_child_sibling.keys.len() >= B {
                            Self::delete_sibling(node, idx, idx - 1);
                        } else if right_child_sibling.keys.len() >= B {
                            Self::delete_sibling(node, idx, idx + 1);
                        } else {
                            Self::delete_merge(node, idx, idx + 1);
                            merged = true;
                        }
                    } else if idx == 0 {
                        let right_child_sibling = node.get_children().get_copy(idx + 1).unwrap();

                        if right_child_sibling.keys.len() >= B {
                            Self::delete_sibling(node, idx, idx + 1);
                        } else {
                            Self::delete_merge(node, idx, idx + 1);
                            merged = true;
                        }
                    } else if idx + 1 == node.get_children().len() {
                        let left_child_sibling = node.get_children().get_copy(idx - 1).unwrap();

                        if left_child_sibling.keys.len() >= B {
                            Self::delete_sibling(node, idx, idx - 1);
                        } else {
                            Self::delete_merge(node, idx, idx - 1);
                            merged = true;
                        }
                    }

                    if merged {
                        return Self::_delete(node, key);
                    }

                    let mut child = node.get_children().get_copy(idx).unwrap();
                    let res = Self::_delete(&mut child, key);
                    node.get_children_mut().replace(idx, child);

                    res
                }
            }
        }
    }

    fn delete_internal_node(node: &mut BTreeNode<K, V, AK, AV>, key: &K, idx: usize) -> Option<V> {
        let mut left_child = node.get_children().get_copy(idx).unwrap();
        let mut right_child = node.get_children().get_copy(idx + 1).unwrap();

        if left_child.keys.len() >= B {
            let (k, v) = Self::delete_predecessor(&mut left_child);
            let v = node.values.replace(idx, v);

            node.keys.replace(idx, k);
            node.get_children_mut().replace(idx, left_child);

            Some(v)
        } else if right_child.keys.len() >= B {
            let (k, v) = Self::delete_successor(&mut right_child);
            let v = node.values.replace(idx, v);

            node.keys.replace(idx, k);
            node.get_children_mut().replace(idx + 1, right_child);

            Some(v)
        } else {
            Self::delete_merge(node, idx, idx + 1);
            Self::_delete(node, key)
        }
    }

    fn delete_predecessor(child: &mut BTreeNode<K, V, AK, AV>) -> (K, V) {
        if child.is_leaf {
            let k = child.keys.pop().unwrap();
            let v = child.values.pop().unwrap();

            return (k, v);
        }

        let n = child.keys.len() - 1;
        let grand_child = child.get_children().get_copy(n).unwrap();

        if grand_child.keys.len() >= B {
            Self::delete_sibling(child, n + 1, n);
        } else {
            Self::delete_merge(child, n + 1, n);
        }

        let mut grand_child = child.get_children().get_copy(n).unwrap();
        let res = Self::delete_predecessor(&mut grand_child);

        child.get_children_mut().replace(n, grand_child);

        res
    }

    fn delete_successor(child: &mut BTreeNode<K, V, AK, AV>) -> (K, V) {
        if child.is_leaf {
            let k = child.keys.remove(0);
            let v = child.values.remove(0);

            return (k, v);
        }

        let grand_child = child.get_children().get_copy(0).unwrap();

        if grand_child.keys.len() >= B {
            Self::delete_sibling(child, 0, 1);
        } else {
            Self::delete_merge(child, 0, 1);
        }

        let mut grand_child = child.get_children().get_copy(0).unwrap();
        let res = Self::delete_successor(&mut grand_child);

        child.get_children_mut().replace(0, grand_child);

        res
    }

    fn delete_merge(node: &mut BTreeNode<K, V, AK, AV>, i: usize, j: usize) {
        let mut child = node.get_children().get_copy(i).unwrap();

        if j > i {
            let child_right_sibling = node.get_children_mut().remove(j);
            child.keys.push(node.keys.remove(i));
            child.values.push(node.values.remove(i));

            child.keys.extend_from(&child_right_sibling.keys);
            child.values.extend_from(&child_right_sibling.values);
            child
                .get_children_mut()
                .extend_from(child_right_sibling.get_children());

            unsafe { child_right_sibling.drop() };

            if node.is_root && node.keys.is_empty() {
                child.is_root = true;
                *node = child;
            } else {
                node.get_children_mut().replace(i, child);
            }
        } else {
            let mut child_left_sibling = node.get_children().get_copy(j).unwrap();
            child_left_sibling.keys.push(node.keys.remove(j));
            child_left_sibling.values.push(node.values.remove(j));

            child_left_sibling.keys.extend_from(&child.keys);
            child_left_sibling.values.extend_from(&child.values);
            child_left_sibling
                .get_children_mut()
                .extend_from(child.get_children());

            let child = node.get_children_mut().remove(i);
            unsafe { child.drop() };

            if node.is_root && node.keys.is_empty() {
                child_left_sibling.is_root = true;
                *node = child_left_sibling;
            } else {
                node.get_children_mut().replace(j, child_left_sibling);
            }
        };
    }

    fn delete_sibling(node: &mut BTreeNode<K, V, AK, AV>, i: usize, j: usize) {
        let mut child = node.get_children().get_copy(i).unwrap();

        if j > i {
            let mut child_right_sibling = node.get_children().get_copy(j).unwrap();

            child.keys.push(node.keys.remove(i));
            child.values.push(node.values.remove(i));

            node.keys.insert(i, child_right_sibling.keys.remove(0));
            node.values.insert(i, child_right_sibling.values.remove(0));

            if !child_right_sibling.get_children().is_empty() {
                child
                    .get_children_mut()
                    .push(child_right_sibling.get_children_mut().remove(0));
            }

            node.get_children_mut().replace(j, child_right_sibling);
        } else {
            let mut child_left_sibling = node.get_children().get_copy(j).unwrap();

            child.keys.insert(0, node.keys.remove(i - 1));
            child.values.insert(0, node.values.remove(i - 1));

            node.keys
                .insert(i - 1, child_left_sibling.keys.pop().unwrap());
            node.values
                .insert(i - 1, child_left_sibling.values.pop().unwrap());

            if !child_left_sibling.get_children().is_empty() {
                child
                    .get_children_mut()
                    .insert(0, child_left_sibling.get_children_mut().pop().unwrap())
            }

            node.get_children_mut().replace(j, child_left_sibling);
        }

        node.get_children_mut().replace(i, child);
    }
}

impl<AK, AV, K, V> Default for SBTreeMap<K, V, AK, AV> {
    fn default() -> Self {
        SBTreeMap::<K, V, AK, AV>::new()
    }
}

impl<'a, K, V, AK, AV> Readable<'a, LittleEndian> for SBTreeMap<K, V, AK, AV> {
    fn read_from<R: Reader<'a, LittleEndian>>(
        reader: &mut R,
    ) -> Result<Self, <speedy::LittleEndian as Context>::Error> {
        let root = BTreeNode::<K, V, AK, AV>::read_from(reader)?;
        let len = reader.read_u64()?;

        Ok(Self { root, len })
    }
}

impl<K, V, AK, AV> Writable<LittleEndian> for SBTreeMap<K, V, AK, AV>
where
    [(); size_of::<BTreeNode<K, V, AK, AV>>()]: Sized,
{
    fn write_to<W: ?Sized + Writer<LittleEndian>>(
        &self,
        writer: &mut W,
    ) -> Result<(), <speedy::LittleEndian as Context>::Error> {
        self.root.write_to(writer)?;
        writer.write_u64(self.len)
    }
}

pub struct BTreeNode<K, V, AK, AV> {
    is_leaf: bool,
    is_root: bool,
    keys: SVec<K, AK>,
    values: SVec<V, AV>,
    children: [u8; size_of::<SVec<(), ()>>()],
}

impl<K, V, AK, AV> Default for BTreeNode<K, V, AK, AV> {
    fn default() -> Self {
        Self::new(true, true)
    }
}

impl<K, V, AK, AV> BTreeNode<K, V, AK, AV> {
    pub fn new(is_leaf: bool, is_root: bool) -> Self {
        let children = SVec::<(), ()>::new();
        let children_bytes = SVec::<(), ()>::to_u8_fixed_size_array(children);

        Self {
            is_root,
            is_leaf,
            keys: SVec::new(),
            values: SVec::new(),
            children: children_bytes,
        }
    }

    pub unsafe fn drop(self) {
        self.keys.drop();
        self.values.drop();

        let children = SVec::<(), ()>::from_u8_fixed_size_array(self.children);
        children.drop();
    }

    #[inline]
    pub fn get_children(&self) -> &SVec<Self, Self> {
        unsafe { &*(self.children.as_ptr() as *const SVec<Self, Self>) }
    }

    #[inline]
    pub fn get_children_mut(&mut self) -> &mut SVec<Self, Self> {
        unsafe { &mut *(self.children.as_mut_ptr() as *mut SVec<Self, Self>) }
    }
}

impl<K, V, AK, AV> StackAllocated<Self, Self> for BTreeNode<K, V, AK, AV> {
    fn size_of_u8_array() -> usize {
        size_of::<Self>()
    }

    fn fixed_size_u8_array() -> [u8; size_of::<Self>()] {
        [0u8; size_of::<Self>()]
    }

    #[inline]
    fn to_u8_fixed_size_array(it: Self) -> [u8; size_of::<Self>()] {
        unsafe { std::mem::transmute(it) }
    }

    #[inline]
    fn from_u8_fixed_size_array(arr: [u8; size_of::<Self>()]) -> Self {
        unsafe { std::mem::transmute(arr) }
    }
}

impl<'a, K, V, AK, AV> Readable<'a, LittleEndian> for BTreeNode<K, V, AK, AV> {
    fn read_from<R: Reader<'a, LittleEndian>>(
        reader: &mut R,
    ) -> Result<Self, <speedy::LittleEndian as Context>::Error> {
        let is_leaf_byte = reader.read_u8()?;
        let is_leaf = match is_leaf_byte {
            0 => false,
            1 => true,
            _ => unreachable!(),
        };
        let is_root_byte = reader.read_u8()?;
        let is_root = match is_root_byte {
            0 => false,
            1 => true,
            _ => unreachable!(),
        };

        let keys = SVec::read_from(reader)?;
        let values = SVec::read_from(reader)?;
        let children = SVec::read_from(reader)?;

        let mut children_bytes = SVec::<(), ()>::to_u8_fixed_size_array(children);

        Ok(Self {
            is_leaf,
            is_root,
            keys,
            values,
            children: children_bytes,
        })
    }
}

impl<K, V, AK, AV> Writable<LittleEndian> for BTreeNode<K, V, AK, AV>
where
    [(); size_of::<Self>()]: Sized,
{
    fn write_to<T: ?Sized + Writer<LittleEndian>>(
        &self,
        writer: &mut T,
    ) -> Result<(), <speedy::LittleEndian as Context>::Error> {
        let is_leaf_byte: u8 = u8::from(self.is_leaf);
        writer.write_u8(is_leaf_byte)?;

        let is_root_byte: u8 = u8::from(self.is_root);
        writer.write_u8(is_root_byte)?;

        self.keys.write_to(writer)?;
        self.values.write_to(writer)?;
        self.get_children().write_to(writer)
    }
}

#[cfg(test)]
mod tests {
    use crate::collections::btree_map::{BTreeNode, SBTreeMap};
    use crate::primitive::StackAllocated;
    use crate::utils::isoprint;
    use crate::{init_allocator, stable};
    use std::fmt::Debug;
    use std::mem::size_of;

    #[ignore]
    fn btree_to_sorted_vec<AK, AV, K: Ord + StackAllocated<K, AK>, V: StackAllocated<V, AV>>(
        btree_node: &BTreeNode<K, V, AK, AV>,
        vec: &mut Vec<(K, V)>,
    ) where
        [(); size_of::<BTreeNode<K, V, AK, AV>>()]: Sized,
        [(); size_of::<AK>()]: Sized,
        [(); size_of::<AV>()]: Sized,
    {
        for i in 0..btree_node.keys.len() {
            if let Some(child) = btree_node.get_children().get_copy(i) {
                btree_to_sorted_vec(&child, vec);
            }
            let k = btree_node.keys.get_copy(i).unwrap();
            let v = btree_node.values.get_copy(i).unwrap();

            vec.push((k, v));
        }

        if let Some(child) = btree_node.get_children().get_copy(btree_node.keys.len()) {
            btree_to_sorted_vec(&child, vec);
        }
    }

    #[ignore]
    fn print_btree<
        AK,
        AV,
        K: Ord + StackAllocated<K, AK> + Debug,
        V: StackAllocated<V, AV> + Debug,
    >(
        btree: &SBTreeMap<K, V, AK, AV>,
    ) where
        [(); size_of::<BTreeNode<K, V, AK, AV>>()]: Sized,
        [(); size_of::<AK>()]: Sized,
        [(); size_of::<AV>()]: Sized,
    {
        let mut nodes_1 = print_btree_level(&btree.root);
        isoprint("");

        loop {
            let mut nodes_2 = vec![];

            for node in &nodes_1 {
                let res = print_btree_level(node);

                for n in res {
                    nodes_2.push(n);
                }
            }

            isoprint("");

            if nodes_2.is_empty() {
                break;
            }

            nodes_1 = nodes_2;
        }
    }

    #[ignore]
    fn print_btree_level<
        AK,
        AV,
        K: Ord + StackAllocated<K, AK> + Debug,
        V: StackAllocated<V, AV> + Debug,
    >(
        btree_node: &BTreeNode<K, V, AK, AV>,
    ) -> Vec<BTreeNode<K, V, AK, AV>>
    where
        [(); size_of::<BTreeNode<K, V, AK, AV>>()]: Sized,
        [(); size_of::<AK>()]: Sized,
        [(); size_of::<AV>()]: Sized,
    {
        let mut children = vec![];

        let keys: Vec<_> = Vec::from(&btree_node.keys);
        let values: Vec<_> = Vec::from(&btree_node.values);

        print!(
            "( is_leaf: {}, is_root: {} - {:?} )",
            btree_node.is_leaf,
            btree_node.is_root,
            keys.iter().zip(values.iter()).collect::<Vec<_>>()
        );

        for ch in 0..btree_node.get_children().len() {
            let child = btree_node.get_children().get_copy(ch).unwrap();

            children.push(child);
        }

        children
    }

    #[test]
    fn random_works_as_expected() {
        stable::clear();
        stable::grow(1).unwrap();
        init_allocator(0);

        let example = vec![
            (10, 1),
            (20, 2),
            (30, 3),
            (40, 4),
            (50, 5),
            (60, 6),
            (70, 7),
            (80, 8),
            (90, 9),
        ];

        let mut map = SBTreeMap::new();

        println!("INSERTION");

        assert!(map.insert(30, 3).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(90, 9).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(10, 1).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(70, 7).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(80, 8).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(50, 5).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(20, 2).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(60, 6).is_none());
        print_btree(&map);
        println!();

        assert!(map.insert(40, 4).is_none());
        print_btree(&map);
        println!();

        assert_eq!(map.len(), 9);

        let mut probe = vec![];
        btree_to_sorted_vec(&map.root, &mut probe);
        assert_eq!(example, probe);

        println!("DELETION");

        assert_eq!(map.remove(&30).unwrap(), 3);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&70).unwrap(), 7);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&50).unwrap(), 5);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&40).unwrap(), 4);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&60).unwrap(), 6);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&20).unwrap(), 2);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&80).unwrap(), 8);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&10).unwrap(), 1);
        print_btree(&map);
        println!();

        assert_eq!(map.remove(&90).unwrap(), 9);
        print_btree(&map);
        println!();

        assert!(map.is_empty());

        unsafe { map.drop() };
    }

    #[test]
    fn sequential_works_as_expected() {
        stable::clear();
        stable::grow(1).unwrap();
        init_allocator(0);

        let mut map = SBTreeMap::new();

        println!("INSERTION");

        for i in 0..10 {
            map.insert(i, 0);
            print_btree(&map);
            println!();
        }

        println!("DELETION");

        for i in 0..10 {
            map.remove(&i).unwrap();
            print_btree(&map);
            println!();
        }

        unsafe { map.drop() };
    }

    #[test]
    fn basic_flow_works_fine() {
        stable::clear();
        stable::grow(1).unwrap();
        init_allocator(0);

        let mut map = SBTreeMap::new();

        let prev = map.insert(1, 10);
        assert!(prev.is_none());

        let val = map.get_copy(&1).unwrap();
        assert_eq!(val, 10);
        assert!(map.contains_key(&1));

        assert!(map.insert(2, 20).is_none());
        map.insert(3, 30);
        map.insert(4, 40);
        map.insert(5, 50);

        let val = map.insert(3, 130).unwrap();
        assert_eq!(val, 30);

        assert!(!map.contains_key(&99));
        assert!(map.remove(&99).is_none());

        unsafe { map.drop() };

        let _map = SBTreeMap::<u64, u64, [u8; 8], [u8; 8]>::default();
    }

    #[test]
    fn deletion_works_fine() {
        stable::clear();
        stable::grow(1).unwrap();
        init_allocator(0);

        let mut map = SBTreeMap::new();

        for i in 0..50 {
            map.insert(i + 10, i);
        }

        let val = map.insert(13, 130).unwrap();
        assert_eq!(val, 3);

        let val1 = map.get_copy(&13).unwrap();
        assert_eq!(val1, 130);

        assert!(!map.contains_key(&99));
        assert!(map.remove(&99).is_none());

        map.insert(13, 3);
        assert_eq!(map.remove(&16).unwrap(), 6);

        map.insert(16, 6);
        map.insert(9, 90);

        assert_eq!(map.remove(&16).unwrap(), 6);

        map.insert(16, 6);
        assert_eq!(map.remove(&9).unwrap(), 90);
        assert_eq!(map.remove(&53).unwrap(), 43);

        map.insert(60, 70);
        map.insert(61, 71);
        assert_eq!(map.remove(&58).unwrap(), 48);

        unsafe { map.drop() };

        let mut map = SBTreeMap::new();

        for i in 0..50 {
            map.insert(i * 2, i);
        }

        map.insert(85, 1);
        assert_eq!(map.remove(&88).unwrap(), 44);

        unsafe { map.drop() };

        let mut map = SBTreeMap::new();

        for i in 0..50 {
            map.insert(i * 2, i);
        }

        map.remove(&94);
        map.remove(&96);
        map.remove(&98);

        assert_eq!(map.remove(&88).unwrap(), 44);

        map.insert(81, 1);
        map.insert(83, 1);
        map.insert(94, 1);
        map.insert(85, 1);

        assert_eq!(map.remove(&86).unwrap(), 43);

        map.insert(71, 1);
        map.insert(73, 1);
        map.insert(75, 1);
        map.insert(77, 1);
        map.insert(79, 1);

        map.insert(47, 1);
        map.insert(49, 1);
        map.insert(51, 1);
        map.insert(53, 1);
        map.insert(55, 1);
        map.insert(57, 1);
        map.insert(59, 1);
        map.insert(61, 1);
        map.insert(63, 1);
        map.insert(65, 1);
        map.insert(67, 1);
        map.insert(69, 1);

        print_btree(&map);

        unsafe { map.drop() };

        let mut map = SBTreeMap::new();

        for i in 150..300 {
            map.insert(i, i);
        }
        for i in 0..150 {
            map.insert(i, i);
        }

        assert_eq!(map.remove(&203).unwrap(), 203);
        assert_eq!(map.remove(&80).unwrap(), 80);

        print_btree(&map);

        unsafe { map.drop() };
    }

    #[test]
    fn complex_deletes_work_fine() {
        stable::clear();
        stable::grow(1).unwrap();
        init_allocator(0);

        let mut map = SBTreeMap::new();

        for i in 0..75 {
            map.insert(i, i);
        }

        for i in 0..75 {
            map.insert(150 - i, i);
        }

        for i in 0..150 {
            let j = if i % 2 == 0 { i } else { 150 - i };

            if j % 3 == 0 {
                map.remove(&j);
            }
        }

        unsafe { map.drop() };

        let mut map = SBTreeMap::new();

        for i in 0..150 {
            map.insert(150 - i, i);
        }

        for i in 0..150 {
            map.remove(&(150 - i));
        }

        unsafe { map.drop() };
    }

    #[test]
    fn set_like_map_works_fine() {
        stable::clear();
        stable::grow(1).unwrap();
        init_allocator(0);

        let mut map = SBTreeMap::<i32, (), i32, ()>::new();
        map.insert(1, ());
        unsafe { map.drop() };
    }
}