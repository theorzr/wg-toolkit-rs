//! Packed XML untyped value.

use std::ops::Index;
use std::slice;

use glam::{Affine3A, Vec2, Vec3, Vec4};
use smallvec::SmallVec;


/// A packed XML untyped value.
#[derive(Debug, Clone)]
pub enum Value {
    Element(Box<Element>),
    String(String),
    Integer(i64),
    Boolean(bool),
    Vector(Vector),
}

/// A packed XML f32 vector of values, this may contains one value or more.
#[derive(Debug, Clone)]
pub struct Vector(SmallVec<[f32; 3]>);

/// A packed element.
#[derive(Debug, Clone)]
pub struct Element {
    /// Proper value of a element.
    pub value: Value,
    /// Children values, each value is mapped to a name that
    /// is not guaranteed to be unique.
    children: SmallVec<[(String, Value); 8]>,
}

impl Element {

    pub fn new() -> Self {
        Self {
            value: Value::default(),
            children: SmallVec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.children.len()
    }

    pub fn iter_children_all(&self) -> impl Iterator<Item = &'_ (String, Value)> + DoubleEndedIterator {
        self.children.iter()
    }

    pub fn iter_children_all_mut(&mut self) -> impl Iterator<Item = &'_ mut (String, Value)> + DoubleEndedIterator {
        self.children.iter_mut()
    }

    pub fn add_children<S: Into<String>>(&mut self, key: S, value: Value) {
        self.children.push((key.into(), value));
    }

    pub fn iter_children<'k, 's: 'k>(&'s self, key: &'k str) -> impl Iterator<Item = &'s Value> + DoubleEndedIterator + 'k {
        self.children.iter().filter_map(move |(k, v)| (k == key).then_some(v))
    }

    pub fn iter_children_mut<'k, 's: 'k>(&'s mut self, key: &'k str) -> impl Iterator<Item = &'s mut Value> + DoubleEndedIterator + 'k {
        self.children.iter_mut().filter_map(move |(k, v)| (k == key).then_some(v))
    }

    pub fn get_child<'a, 'b>(&'a self, key: &'b str) -> Option<&'a Value> {
        self.children.iter().find_map(|(k, v)| (k == key).then_some(v))
    }

    pub fn get_child_mut<'a, 'b>(&'a mut self, key: &'b str) -> Option<&'a mut Value> {
        self.children.iter_mut().find_map(|(k, v)| (k == key).then_some(v))
    }

    pub fn insert_child(&mut self, index: usize, name: String, value: Value) -> &'_ mut Value {
        self.children.insert(index, (name, value));
        &mut self.children[index].1
    }

    pub fn push_child(&mut self, name: String, value: Value) -> &'_ mut Value {
        self.insert_child(self.children.len(), name, value)
    }

}

impl Value {

    /// Try to get this value as an element if possible.
    #[inline]
    pub fn as_element(&self) -> Option<&Element> {
        if let Self::Element(elt) = self { Some(&**elt) } else { None }
    }

    /// Try to get this value as a string if possible.
    #[inline]
    pub fn as_string(&self) -> Option<&str> {
        if let Self::String(s) = self { Some(s.as_str()) } else { None }
    }

    /// Try to get this value as an integer if possible.
    #[inline]
    pub fn as_integer(&self) -> Option<i64> {
        if let Self::Integer(n) = *self { Some(n) } else { None }
    }

    /// Try to get this value as a boolean is possible.
    #[inline]
    pub fn as_boolean(&self) -> Option<bool> {
        if let Self::Boolean(b) = *self { Some(b) } else { None }
    }

    /// Try to get this value as a boolean is possible.
    #[inline]
    pub fn as_vector(&self) -> Option<&Vector> {
        if let Self::Vector(v) = self { Some(v) } else { None }
    }

    /// Try to get this value as a float if possible.
    ///
    /// If the underlying value is not a float vector, then the value may be interpreted
    /// from an integer, or a string that can be parsed.
    #[inline]
    pub fn as_float(&self) -> Option<f32> {
        match self {
            Self::Vector(v) => v.as_float(),
            &Self::Integer(n) => Some(n as f32),
            Self::String(s) => s.parse::<f32>().ok(),
            _ => None
        }
    }

    /// Try to get this value as a vec2 if possible.
    ///
    /// If the underlying value is not a float vector, then the vector may be interpreted
    /// from a string formatted as space-separated decimal floats.
    #[inline]
    pub fn as_vec2(&self) -> Option<Vec2> {
        match self {
            Self::Vector(v) => v.as_vec2(),
            Self::String(s) => Some(Vec2::from_array(parse_string_vector(&s)?)),
            _ => None
        }
    }

    /// Try to get this value as a vec3 if possible.
    ///
    /// If the underlying value is not a float vector, then the vector may be interpreted
    /// from a string formatted as space-separated decimal floats.
    #[inline]
    pub fn as_vec3(&self) -> Option<Vec3> {
        match self {
            Self::Vector(v) => v.as_vec3(),
            Self::String(s) => Some(Vec3::from_array(parse_string_vector(&s)?)),
            _ => None
        }
    }

    /// Try to get this value as a vec4 if possible.
    ///
    /// If the underlying value is not a float vector, then the vector may be interpreted
    /// from a string formatted as space-separated decimal floats.
    #[inline]
    pub fn as_vec4(&self) -> Option<Vec4> {
        match self {
            Self::Vector(v) => v.as_vec4(),
            Self::String(s) => Some(Vec4::from_array(parse_string_vector(&s)?)),
            _ => None
        }
    }

    /// Try to get this value as an affine3 if possible.
    #[inline]
    pub fn as_affine3(&self) -> Option<Affine3A> {
        self.as_vector()?.as_affine3()
    }

}

/// Default value is an empty string, which do no allocation.
impl Default for Value {
    fn default() -> Self {
        Self::String(String::new())
    }
}

impl Vector {

    /// Build a vector from its raw components.
    #[inline]
    pub(crate) fn from_smallvec(components: SmallVec<[f32; 3]>) -> Self {
        Self(components)
    }

    /// Get the size of this float vector.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Return the vector component at given index.
    #[inline]
    pub fn get(&self, index: usize) -> Option<f32> {
        self.0.get(index).copied()
    }

    /// Return an ordered iterator to the components of this vector.
    #[inline]
    pub fn iter(&self) -> slice::Iter<'_, f32> {
        self.0.iter()
    }

    /// Return a 1-component vector (the scalar) if this vector is of size 1.
    #[inline]
    pub fn as_float(&self) -> Option<f32> {
        match self.0[..] {
            [x] => Some(x),
            _ => None,
        }
    }

    /// Return a 2-component vector if this vector is of size 2.
    #[inline]
    pub fn as_vec2(&self) -> Option<Vec2> {
        match self.0[..] {
            [x, y] => Some(Vec2::new(x, y)),
            _ => None,
        }
    }

    /// Return a 3-component vector if this vector is of size 3.
    #[inline]
    pub fn as_vec3(&self) -> Option<Vec3> {
        match self.0[..] {
            [x, y, z] => Some(Vec3::new(x, y, z)),
            _ => None,
        }
    }

    /// Return a 4-component vector if this vector is of size 4.
    #[inline]
    pub fn as_vec4(&self) -> Option<Vec4> {
        match self.0[..] {
            [x, y, z, w] => Some(Vec4::new(x, y, z, w)),
            _ => None,
        }
    }

    /// Return a 3D affine transform if this vector is of size 12.
    #[inline]
    pub fn as_affine3(&self) -> Option<Affine3A> {
        if self.0.len() == 12 {
            Some(Affine3A::from_cols_slice(&self.0[..]))
        } else {
            None
        }
    }

}

impl Index<usize> for Vector {

    type Output = f32;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }

}

fn parse_string_vector<const LEN: usize>(s: &str) -> Option<[f32; LEN]> {
    let mut ret = [0.0; LEN];
    for (i, part) in s.splitn(LEN, ' ').enumerate() {
        ret[i] = part.parse::<f32>().ok()?;
    }
    Some(ret)
}
