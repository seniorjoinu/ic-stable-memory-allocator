#[cfg(test)]
mod btree_map_benchmark {
    use crate::collections::certified_btree_map::SCertifiedBTreeMap;
    use crate::utils::certification::{AsHashTree as MyAsHashTree, AsHashableBytes};
    use crate::{init_allocator, measure, stable};
    use ic_certified_map::{leaf_hash, AsHashTree, Hash, HashTree, RbTree};
    use rand::seq::SliceRandom;
    use rand::thread_rng;
    use std::borrow::Cow;

    const ITERATIONS: usize = 10_000;

    struct Val(usize);

    impl AsHashTree for Val {
        fn root_hash(&self) -> Hash {
            leaf_hash(&self.0.to_le_bytes())
        }

        fn as_hash_tree(&self) -> HashTree<'_> {
            HashTree::Leaf(Cow::Owned(self.0.to_le_bytes().to_vec()))
        }
    }

    impl AsHashableBytes for usize {
        fn as_hashable_bytes(&self) -> Vec<u8> {
            self.to_le_bytes().to_vec()
        }

        fn from_hashable_bytes(bytes: Vec<u8>) -> Self {
            usize::from_le_bytes(bytes.try_into().unwrap())
        }
    }

    #[test]
    #[ignore]
    fn body_direct() {
        let mut example = Vec::new();
        for i in 0..ITERATIONS {
            example.push(i);
        }
        example.shuffle(&mut thread_rng());

        {
            let mut rbtree_map = RbTree::new();

            measure!("RBTree map insert", ITERATIONS, {
                for i in 0..ITERATIONS {
                    rbtree_map.insert(example[i].to_le_bytes(), Val(example[i]));
                }
            });

            measure!("RBTree map search", ITERATIONS, {
                for i in 0..ITERATIONS {
                    rbtree_map.get(&example[i].to_le_bytes()).unwrap();
                }
            });

            measure!("RBTree map witness", ITERATIONS, {
                for i in 0..ITERATIONS {
                    rbtree_map.witness(&example[i].to_le_bytes());
                }
            });

            measure!("RBTree map remove", ITERATIONS, {
                for i in 0..ITERATIONS {
                    rbtree_map.delete(&example[i].to_le_bytes());
                }
            });
        }

        {
            stable::clear();
            stable::grow(1).unwrap();
            init_allocator(0);

            let mut stable_certified_btree_map = SCertifiedBTreeMap::new();

            measure!("Stable certified btree map insert", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_certified_btree_map.insert(example[i], example[i]);
                }
            });

            measure!("Stable certified btree map search", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_certified_btree_map.get_copy(&example[i]).unwrap();
                }
            });

            measure!("Stable certified btree map witness", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_certified_btree_map.witness(&example[i], None);
                }
            });

            measure!("Stable certified btree map remove", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_certified_btree_map.remove(&example[i]).unwrap();
                }
            });
        }
    }
}