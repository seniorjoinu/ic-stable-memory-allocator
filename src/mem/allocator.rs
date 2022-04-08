use crate::utils::mem_context::{stable, OutOfMemory, PAGE_SIZE_BYTES};
use std::mem::size_of;
use crate::mem::membox::common::{MEM_BOX_MIN_SIZE, Side, Size, Word};
use crate::MemBox;
use crate::utils::math::fast_log2;

pub(crate) const EMPTY_PTR: Word = Word::MAX;
pub(crate) const MAGIC: [u8; 4] = [b'S', b'M', b'A', b'M'];
pub(crate) const SEG_CLASS_PTRS_COUNT: Size = Size::BITS as Size - 4;

pub(crate) type SegClassId = u32;

#[derive(Debug, Copy, Clone)]
pub(crate) struct Free;

impl MemBox<Free> {
    pub(crate) fn set_prev_free_ptr(&mut self, prev_ptr: Word) {
        self.assert_allocated(false, None);

        self._write_word(0, prev_ptr);
    }

    pub(crate) fn get_prev_free_ptr(&self) -> Word {
        self.assert_allocated(false, None);

        self._read_word(0)
    }

    pub(crate) fn set_next_free_ptr(&mut self, next_ptr: Word) {
        self.assert_allocated(false, None);

        self._write_word(size_of::<Word>(), next_ptr);
    }

    pub(crate) fn get_next_free_ptr(&self) -> Word {
        self.assert_allocated(false, None);

        self._read_word(size_of::<Word>())
    }
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct StableMemoryAllocator;

impl MemBox<StableMemoryAllocator> {
    const SIZE: Size =
        MAGIC.len() + SEG_CLASS_PTRS_COUNT * size_of::<Word>() + size_of::<Word>() * 2;

    /// # Safety
    pub(crate) unsafe fn init(offset: Word) -> Self {
        let mut allocator = MemBox::<StableMemoryAllocator>::init(offset, Self::SIZE, true);

        allocator._write_bytes(0, &MAGIC);
        allocator.reset();

        allocator
    }

    /// # Safety
    pub(crate) unsafe fn reinit(offset: Word) -> Option<Self> {
        let membox = MemBox::<StableMemoryAllocator>::from_ptr(offset, Side::Start)?;
        let (size, allocated) = membox.get_meta();
        if !allocated || size != Self::SIZE {
            return None;
        }

        let mut magic = [0u8; MAGIC.len()];
        membox._read_bytes(0, &mut magic);
        if magic != MAGIC {
            return None;
        }

        Some(membox)
    }

    pub(crate) fn allocate<T>(&mut self, mut size: Size) -> Result<MemBox<T>, OutOfMemory> {
        if size < MEM_BOX_MIN_SIZE {
            size = MEM_BOX_MIN_SIZE
        }

        let free_membox = self.pop_allocated_membox(size)?;
        let membox = unsafe { MemBox::<T>::from_ptr(free_membox.get_ptr(), Side::Start).unwrap() };
        let membox_size = membox.get_meta().0;

        let total_free = self.get_free_size();
        self.set_free_size(total_free - membox_size as Word);

        let total_allocated = self.get_allocated_size();
        self.set_allocated_size(total_allocated + membox_size as Word);

        Ok(membox)
    }

    pub(crate) fn deallocate<T>(&mut self, mut membox: MemBox<T>) {
        let (size, allocated) = membox.get_meta();
        membox.assert_allocated(true, Some(allocated));
        membox.set_allocated(false);

        let total_free = self.get_free_size();
        self.set_free_size(total_free + size as Word);

        let total_allocated = self.get_allocated_size();
        self.set_allocated_size(total_allocated - size as Word);

        let membox = unsafe { MemBox::<Free>::from_ptr(membox.get_ptr(), Side::Start).unwrap() };
        self.push_free_membox(membox);
    }

    pub(crate) fn reallocate<T>(
        &mut self,
        membox: MemBox<T>,
        new_size: Size,
    ) -> Result<MemBox<T>, OutOfMemory> {
        let mut data = vec![0u8; membox.get_meta().0];
        membox._read_bytes(0, &mut data);

        self.deallocate(membox);
        let mut new_membox = self.allocate(new_size)?;
        new_membox._write_bytes(0, &data);

        Ok(new_membox)
    }

    pub(crate) fn reset(&mut self) {
        let empty_ptr_bytes = EMPTY_PTR.to_le_bytes();

        for i in 0..SEG_CLASS_PTRS_COUNT {
            self._write_bytes(MAGIC.len() + i * size_of::<Word>(), &empty_ptr_bytes)
        }

        self.set_allocated_size(0);
        self.set_free_size(0);

        let total_free_size =
            stable::size_pages() * PAGE_SIZE_BYTES as Word - self.get_next_neighbor_ptr();

        if total_free_size > 0 {
            let ptr = self.get_next_neighbor_ptr();

            let free_mem_box =
                unsafe { MemBox::<Free>::new_total_size(ptr, total_free_size as Size, false) };

            self.set_free_size(free_mem_box.get_meta().0 as Word);

            self.push_free_membox(free_mem_box);
        }
    }

    fn push_free_membox(&mut self, mut membox: MemBox<Free>) {
        membox.assert_allocated(false, None);

        membox = self.maybe_merge_with_free_neighbors(membox);
        let (size, _) = membox.get_meta();

        let seg_class_id = get_seg_class_id(size);

        let head_opt = unsafe { self.get_seg_class_head(seg_class_id) };

        self.set_seg_class_head(seg_class_id, membox.get_ptr());
        membox.set_prev_free_ptr(self.get_ptr());

        match head_opt {
            None => {
                membox.set_next_free_ptr(EMPTY_PTR);
            }
            Some(mut head) => {
                membox.set_next_free_ptr(head.get_ptr());

                head.set_prev_free_ptr(membox.get_ptr());
            }
        }
    }

    /// returns ALLOCATED membox
    fn pop_allocated_membox(&mut self, size: Size) -> Result<MemBox<Free>, OutOfMemory> {
        let mut seg_class_id = get_seg_class_id(size);
        let free_membox_opt = unsafe { self.get_seg_class_head(seg_class_id) };

        // iterate over this seg class, until big enough membox found or til it ends
        if let Some(mut free_membox) = free_membox_opt {
            loop {
                let (membox_size, _) = free_membox.get_meta();

                // if valid membox found,
                if membox_size >= size {
                    self.eject_from_freelist(seg_class_id, &mut free_membox);

                    free_membox.set_allocated(true);

                    return Ok(free_membox);
                }

                let next_ptr = free_membox.get_next_free_ptr();
                if next_ptr == EMPTY_PTR {
                    break;
                }

                free_membox = unsafe { MemBox::<Free>::from_ptr(next_ptr, Side::Start).unwrap() };
            }
        }

        // if no appropriate membox was found previously, try to find any of larger size
        let mut free_membox_opt = None;
        seg_class_id += 1;

        while seg_class_id < SEG_CLASS_PTRS_COUNT as u32 {
            free_membox_opt = unsafe { self.get_seg_class_head(seg_class_id) };

            if let Some(free_membox) = &free_membox_opt {
                if free_membox.get_meta().0 >= size {
                    break;
                }
            }

            seg_class_id += 1;
        }

        match free_membox_opt {
            // if at least one such a big membox found, pop it, split in two, take first, push second
            Some(mut free_membox) => {
                self.eject_from_freelist(seg_class_id, &mut free_membox);

                let res = unsafe { free_membox.split(size) };
                match res {
                    Ok((mut result, additional)) => {
                        result.set_allocated(true);
                        self.push_free_membox(additional);

                        Ok(result)
                    }
                    Err(mut result) => {
                        result.set_allocated(true);

                        Ok(result)
                    }
                }
            }
            // otherwise, grow and if grown successfully, split in two, take first, push second
            None => {
                let pages_to_grow = size / PAGE_SIZE_BYTES + 1;
                let prev_total_size = stable::grow(pages_to_grow as u64)? * PAGE_SIZE_BYTES as Word;

                let total_free_size =
                    stable::size_pages() * PAGE_SIZE_BYTES as Word - prev_total_size;

                let ptr = prev_total_size;

                let new_free_membox =
                    unsafe { MemBox::<Free>::new_total_size(ptr, total_free_size as Size, false) };

                let new_free_membox_size = new_free_membox.get_meta().0;
                let total_free_size = self.get_free_size();
                self.set_free_size(total_free_size + new_free_membox_size as Word);

                match unsafe { new_free_membox.split(size) } {
                    Ok((mut result, additional)) => {
                        result.set_allocated(true);

                        self.push_free_membox(additional);

                        Ok(result)
                    }
                    Err(mut new_free_membox) => {
                        new_free_membox.set_allocated(true);

                        Ok(new_free_membox)
                    }
                }
            }
        }
    }

    pub(crate) fn get_allocated_size(&self) -> Word {
        self._read_word(MAGIC.len() + SEG_CLASS_PTRS_COUNT * size_of::<Word>())
    }

    fn set_allocated_size(&mut self, size: Word) {
        self._write_word(MAGIC.len() + SEG_CLASS_PTRS_COUNT * size_of::<Word>(), size);
    }

    pub(crate) fn get_free_size(&self) -> Word {
        self._read_word(MAGIC.len() + SEG_CLASS_PTRS_COUNT * size_of::<Word>() + size_of::<Word>())
    }

    fn set_free_size(&mut self, size: Word) {
        self._write_word(
            MAGIC.len() + SEG_CLASS_PTRS_COUNT * size_of::<Word>() + size_of::<Word>(),
            size,
        );
    }

    unsafe fn get_seg_class_head(&self, id: SegClassId) -> Option<MemBox<Free>> {
        let ptr = self._read_word(Self::get_seg_class_head_offset(id));
        if ptr == EMPTY_PTR {
            return None;
        }

        Some(MemBox::<Free>::from_ptr(ptr, Side::Start).unwrap())
    }

    fn eject_from_freelist(&mut self, seg_class_id: SegClassId, membox: &mut MemBox<Free>) {
        // if membox is the head of it's seg class
        if membox.get_prev_free_ptr() == self.get_ptr() {
            self.set_seg_class_head(seg_class_id, membox.get_next_free_ptr());

            let next_opt =
                unsafe { MemBox::<Free>::from_ptr(membox.get_next_free_ptr(), Side::Start) };

            if let Some(mut next) = next_opt {
                next.set_prev_free_ptr(self.get_ptr());
            }
        } else {
            let mut prev = unsafe {
                MemBox::<Free>::from_ptr(membox.get_prev_free_ptr(), Side::Start).unwrap()
            };
            let next_opt =
                unsafe { MemBox::<Free>::from_ptr(membox.get_next_free_ptr(), Side::Start) };

            if let Some(mut next) = next_opt {
                prev.set_next_free_ptr(next.get_ptr());
                next.set_prev_free_ptr(prev.get_ptr());
            } else {
                prev.set_next_free_ptr(EMPTY_PTR);
            }
        }

        membox.set_prev_free_ptr(EMPTY_PTR);
        membox.set_next_free_ptr(EMPTY_PTR);
    }

    fn maybe_merge_with_free_neighbors(&mut self, mut membox: MemBox<Free>) -> MemBox<Free> {
        let prev_neighbor_opt = unsafe { membox.get_neighbor(Side::Start) };
        membox = if let Some(mut prev_neighbor) = prev_neighbor_opt {
            let (neighbor_size, neighbor_allocated) = prev_neighbor.get_meta();

            if !neighbor_allocated {
                let seg_class_id = get_seg_class_id(neighbor_size);
                self.eject_from_freelist(seg_class_id, &mut prev_neighbor);

                unsafe { membox.merge_with_neighbor(prev_neighbor) }
            } else {
                membox
            }
        } else {
            membox
        };

        let next_neighbor_opt = unsafe { membox.get_neighbor(Side::End) };
        membox = if let Some(mut next_neighbor) = next_neighbor_opt {
            let (neighbor_size, neighbor_allocated) = next_neighbor.get_meta();

            if !neighbor_allocated {
                let seg_class_id = get_seg_class_id(neighbor_size);
                self.eject_from_freelist(seg_class_id, &mut next_neighbor);

                unsafe { membox.merge_with_neighbor(next_neighbor) }
            } else {
                membox
            }
        } else {
            membox
        };

        membox
    }

    fn set_seg_class_head(&mut self, id: SegClassId, head_ptr: Word) {
        self._write_word(Self::get_seg_class_head_offset(id), head_ptr);
    }

    fn get_seg_class_head_offset(seg_class_id: SegClassId) -> Size {
        assert!(seg_class_id < SEG_CLASS_PTRS_COUNT as SegClassId);

        MAGIC.len() + seg_class_id as Size * size_of::<Word>()
    }
}

fn get_seg_class_id(size: Size) -> SegClassId {
    let mut log = fast_log2(size);

    if 2usize.pow(log) < size {
        log += 1;
    }

    if log > 3 {
        (log - 4) as SegClassId
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use crate::{MemBox, StableMemoryAllocator};
    use crate::mem::allocator::SEG_CLASS_PTRS_COUNT;
    use crate::utils::mem_context::stable;

    #[test]
    fn initialization_works_fine() {
        stable::clear();
        stable::grow(1).expect("Unable to grow");

        unsafe {
            let sma = MemBox::<StableMemoryAllocator>::init(0);
            let free_memboxes: Vec<_> = (0..SEG_CLASS_PTRS_COUNT)
                .filter_map(|it| sma.get_seg_class_head(it as u32))
                .collect();

            assert_eq!(free_memboxes.len(), 1);
            let free_membox1 = free_memboxes[0];
            let (size1, allocated1) = free_membox1.get_meta();

            let sma = MemBox::<StableMemoryAllocator>::reinit(0).unwrap();
            let free_memboxes: Vec<_> = (0..SEG_CLASS_PTRS_COUNT)
                .filter_map(|it| sma.get_seg_class_head(it as u32))
                .collect();

            assert_eq!(free_memboxes.len(), 1);
            let free_membox2 = free_memboxes[0];
            let (size2, allocated2) = free_membox2.get_meta();

            assert_eq!(size1, size2);
            assert_eq!(allocated1, allocated2);
        }
    }

    #[test]
    fn allocation_works_fine() {
        stable::clear();
        stable::grow(1).expect("Unable to grow");

        unsafe {
            let mut sma = MemBox::<StableMemoryAllocator>::init(0);
            let mut memboxes = vec![];

            // try to allocate 1000 MB
            for i in 0..1024 {
                let membox = sma
                    .allocate::<u8>(1024)
                    .unwrap_or_else(|_| panic!("Unable to allocate on step {}", i));

                assert!(membox.get_meta().0 >= 1024, "Invalid membox size at {}", i);

                memboxes.push(membox);
            }

            assert!(sma.get_allocated_size() >= 1024 * 1024);

            for i in 0..1024 {
                let mut membox = memboxes[i];
                membox = sma
                    .reallocate(membox, 2 * 1024)
                    .unwrap_or_else(|_| panic!("Unable to reallocate on step {}", i));

                assert!(
                    membox.get_meta().0 >= 2 * 1024,
                    "Invalid membox size at {}",
                    i
                );

                memboxes[i] = membox;
            }

            assert!(sma.get_allocated_size() >= 2 * 1024 * 1024);

            for i in 0..1024 {
                let membox = memboxes[i];
                sma.deallocate(membox);
            }

            assert_eq!(sma.get_allocated_size(), 0);
        }
    }
}