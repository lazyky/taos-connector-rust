use std::{ffi::c_void, fmt::Debug};

use super::{IsColumnView, Offsets};
use crate::{
    common::{BorrowedValue, Ty},
    prelude::InlinableWrite,
    util::InlineJson,
};

use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct JsonView {
    // version: Version,
    pub offsets: Offsets,
    pub data: Bytes,
}

type View = JsonView;

impl IsColumnView for View {
    fn ty(&self) -> Ty {
        Ty::Json
    }
    fn from_borrowed_value_iter<'b>(_: impl Iterator<Item = BorrowedValue<'b>>) -> Self {
        todo!()
    }
}

impl JsonView {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Check if the value at `row` index is NULL or not.
    pub fn is_null(&self, row: usize) -> bool {
        if row < self.len() {
            unsafe { self.is_null_unchecked(row) }
        } else {
            false
        }
    }

    /// Unsafe version for [methods.is_null]
    pub unsafe fn is_null_unchecked(&self, row: usize) -> bool {
        self.offsets.get_unchecked(row) < 0
    }

    pub unsafe fn get_unchecked(&self, row: usize) -> Option<&InlineJson> {
        let offset = self.offsets.get_unchecked(row);
        if offset >= 0 {
            Some(InlineJson::<u16>::from_ptr(
                self.data.as_ptr().offset(offset as isize),
            ))
        } else {
            None
        }
    }

    pub unsafe fn get_value_unchecked(&self, row: usize) -> BorrowedValue {
        // todo: use simd_json::BorrowedValue as Json.
        self.get_unchecked(row)
            .map(|s| BorrowedValue::Json(s.as_bytes().into()))
            .unwrap_or(BorrowedValue::Null(Ty::Json))
    }

    pub unsafe fn get_raw_value_unchecked(&self, row: usize) -> (Ty, u32, *const c_void) {
        match self.get_unchecked(row) {
            Some(json) => (Ty::Json, json.len() as _, json.as_ptr() as _),
            None => (Ty::Json, 0, std::ptr::null()),
        }
    }

    pub fn slice(&self, mut range: std::ops::Range<usize>) -> Option<Self> {
        if range.start >= self.len() {
            return None;
        }
        if range.end > self.len() {
            range.end = self.len();
        }
        if range.len() == 0 {
            return None;
        }
        let (offsets, range) = unsafe { self.offsets.slice_unchecked(range.clone()) };
        if let Some(range) = range {
            let range = range.0 as usize..range.1.map(|v| v as usize).unwrap_or(self.data.len());
            let data = self.data.slice(range);
            Some(Self { offsets, data })
        } else {
            let data = self.data.slice(0..0);
            Some(Self { offsets, data })
        }
    }

    pub fn iter(&self) -> VarCharIter {
        VarCharIter { view: self, row: 0 }
    }

    pub fn to_vec(&self) -> Vec<Option<String>> {
        (0..self.len())
            .map(|row| unsafe { self.get_unchecked(row) }.map(|s| s.to_string()))
            .collect()
    }

    /// Write column data as raw bytes.
    pub(crate) fn write_raw_into<W: std::io::Write>(&self, mut wtr: W) -> std::io::Result<usize> {
        let mut offsets = Vec::new();
        let mut bytes: Vec<u8> = Vec::new();
        for v in self.iter() {
            if let Some(v) = v {
                offsets.push(bytes.len() as i32);
                bytes.write_inlined_str::<2>(v.as_str()).unwrap();
            } else {
                offsets.push(-1);
            }
        }
        unsafe {
            // dbg!(&offsets);
            let offsets_bytes = std::slice::from_raw_parts(
                offsets.as_ptr() as *const u8,
                offsets.len() * std::mem::size_of::<i32>(),
            );
            wtr.write_all(offsets_bytes)?;
            wtr.write_all(&bytes)?;
            return Ok(offsets_bytes.len() + bytes.len());
        }
        // let offsets = self.offsets.as_bytes();
        // wtr.write_all(offsets)?;
        // wtr.write_all(&self.data)?;
        // Ok(offsets.len() + self.data.len())
    }

    pub fn from_iter<
        S: AsRef<str>,
        T: Into<Option<S>>,
        I: ExactSizeIterator<Item = T>,
        V: IntoIterator<Item = T, IntoIter = I>,
    >(
        iter: V,
    ) -> Self {
        let iter = iter.into_iter();
        let mut offsets = Vec::with_capacity(iter.len());
        let mut data = Vec::new();

        for i in iter.map(|v| v.into()) {
            if let Some(s) = i {
                let s: &str = s.as_ref();
                offsets.push(data.len() as i32);
                data.write_inlined_str::<2>(&s).unwrap();
            } else {
                offsets.push(-1);
            }
        }
        // dbg!(&offsets);
        let offsets_bytes = unsafe {
            Vec::from_raw_parts(
                offsets.as_mut_ptr() as *mut u8,
                offsets.len() * 4,
                offsets.capacity() * 4,
            )
        };
        std::mem::forget(offsets);
        Self {
            offsets: Offsets(offsets_bytes.into()),
            data: data.into(),
        }
    }
}

pub struct VarCharIter<'a> {
    view: &'a JsonView,
    row: usize,
}

impl<'a> Iterator for VarCharIter<'a> {
    type Item = Option<&'a InlineJson>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.row < self.view.len() {
            let row = self.row;
            self.row += 1;
            Some(unsafe { self.view.get_unchecked(row) })
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.row < self.view.len() {
            let len = self.view.len() - self.row;
            (len, Some(len))
        } else {
            (0, Some(0))
        }
    }
}

impl<'a> ExactSizeIterator for VarCharIter<'a> {
    fn len(&self) -> usize {
        self.view.len() - self.row
    }
}

#[test]
fn test_slice() {
    let data = [None, Some(""), Some("abc"), Some("中文"), None, None, Some("a loooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooog string")];
    let view = JsonView::from_iter::<&str, _, _, _>(data);
    let slice = view.slice(0..0);
    assert!(slice.is_none());
    let slice = view.slice(100..1000);
    assert!(slice.is_none());

    for start in 0..data.len() {
        let end = start + 1;
        for end in end..data.len() {
            let slice = view.slice(start..end).unwrap();
            assert_eq!(
                slice.to_vec().as_slice(),
                &itertools::Itertools::collect_vec(
                    data[start..end]
                        .into_iter()
                        .map(|s| s.map(ToString::to_string))
                )
            );
        }
    }
}
