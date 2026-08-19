//! Structure-of-arrays field storage keyed by cell id.
//!
//! Fields live in a `Vec` of (name, values) pairs — not a HashMap — so
//! iteration order is fixed and deterministic. The handful of fields a stage
//! touches makes linear name lookup free in practice.

/// SoA field storage for one grid resolution: f32 fields for continuous
/// quantities, u32 fields for ids and bitmasks (exact equality and bit ops —
/// decision log 2026-08-19).
#[derive(Clone, Default)]
pub struct FieldStore {
    cell_count: u32,
    fields: Vec<(String, Vec<f32>)>,
    fields_u32: Vec<(String, Vec<u32>)>,
}

impl FieldStore {
    pub fn new(cell_count: u32) -> Self {
        FieldStore {
            cell_count,
            fields: Vec::new(),
            fields_u32: Vec::new(),
        }
    }

    #[inline]
    pub fn cell_count(&self) -> u32 {
        self.cell_count
    }

    /// Get a field by name, or insert it zero-filled first. Returns a mutable
    /// slice of exactly `cell_count` values.
    pub fn get_or_insert_mut(&mut self, name: &str) -> &mut [f32] {
        if let Some(idx) = self.fields.iter().position(|(n, _)| n == name) {
            return &mut self.fields[idx].1;
        }
        self.fields
            .push((name.to_string(), vec![0.0; self.cell_count as usize]));
        &mut self.fields.last_mut().unwrap().1
    }

    pub fn get(&self, name: &str) -> Option<&[f32]> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut [f32]> {
        self.fields
            .iter_mut()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_mut_slice())
    }

    /// Replace or insert a whole field. Panics if the length doesn't match the
    /// cell count — a field of the wrong size is always a programming error.
    pub fn set(&mut self, name: &str, values: Vec<f32>) {
        assert_eq!(
            values.len(),
            self.cell_count as usize,
            "field '{name}' has wrong length for this grid"
        );
        if let Some(idx) = self.fields.iter().position(|(n, _)| n == name) {
            self.fields[idx].1 = values;
        } else {
            self.fields.push((name.to_string(), values));
        }
    }

    /// Field names in insertion order (fixed, deterministic).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(|(n, _)| n.as_str())
    }

    // ----- u32 fields (ids, bitmasks) -----

    /// Get a u32 field by name, or insert it zero-filled first.
    pub fn get_or_insert_mut_u32(&mut self, name: &str) -> &mut [u32] {
        if let Some(idx) = self.fields_u32.iter().position(|(n, _)| n == name) {
            return &mut self.fields_u32[idx].1;
        }
        self.fields_u32
            .push((name.to_string(), vec![0; self.cell_count as usize]));
        &mut self.fields_u32.last_mut().unwrap().1
    }

    pub fn get_u32(&self, name: &str) -> Option<&[u32]> {
        self.fields_u32
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    pub fn get_u32_mut(&mut self, name: &str) -> Option<&mut [u32]> {
        self.fields_u32
            .iter_mut()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_mut_slice())
    }

    /// Replace or insert a whole u32 field. Panics on length mismatch, same
    /// contract as [`FieldStore::set`].
    pub fn set_u32(&mut self, name: &str, values: Vec<u32>) {
        assert_eq!(
            values.len(),
            self.cell_count as usize,
            "field '{name}' has wrong length for this grid"
        );
        if let Some(idx) = self.fields_u32.iter().position(|(n, _)| n == name) {
            self.fields_u32[idx].1 = values;
        } else {
            self.fields_u32.push((name.to_string(), values));
        }
    }

    /// u32 field names in insertion order (fixed, deterministic).
    pub fn names_u32(&self) -> impl Iterator<Item = &str> {
        self.fields_u32.iter().map(|(n, _)| n.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_roundtrip_and_fixed_order() {
        let mut fs = FieldStore::new(4);
        fs.get_or_insert_mut("elevation_m")[2] = 5.0;
        fs.set("thickness_km", vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(fs.get("elevation_m").unwrap()[2], 5.0);
        assert_eq!(fs.get("missing"), None);
        let names: Vec<&str> = fs.names().collect();
        assert_eq!(names, vec!["elevation_m", "thickness_km"]);
    }

    #[test]
    fn u32_fields_roundtrip_and_are_separate() {
        let mut fs = FieldStore::new(4);
        fs.set_u32("plate_id", vec![7, 7, 3, 3]);
        fs.get_or_insert_mut_u32("features")[1] = 0b101;
        assert_eq!(fs.get_u32("plate_id").unwrap(), &[7, 7, 3, 3]);
        assert_eq!(fs.get_u32("features").unwrap()[1], 0b101);
        assert_eq!(fs.get_u32("missing"), None);
        // The f32 and u32 namespaces are independent.
        assert_eq!(fs.get("plate_id"), None);
        let names: Vec<&str> = fs.names_u32().collect();
        assert_eq!(names, vec!["plate_id", "features"]);
    }

    #[test]
    #[should_panic]
    fn wrong_length_u32_panics() {
        let mut fs = FieldStore::new(4);
        fs.set_u32("bad", vec![0; 5]);
    }

    #[test]
    #[should_panic]
    fn wrong_length_panics() {
        let mut fs = FieldStore::new(4);
        fs.set("bad", vec![0.0; 3]);
    }
}
