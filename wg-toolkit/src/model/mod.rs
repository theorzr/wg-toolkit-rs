//! Compiled model memory representation, encoding and decoding.

use std::io::{Read, Seek};

use thiserror::Error;

pub mod primitive;
pub mod visual;

use primitive::{PrimitiveReader, Vertices, Indices, Vertex, Primitive, Group};
use visual::{Visual, RenderSet};


/// Decode and resolve a compiled model.
pub fn from_readers<Rv, Rp>(visual_reader: Rv, primitive_reader: Rp) -> Result<Model, DeError>
where
    Rv: Read + Seek,
    Rp: Read + Seek,
{

    let visual = visual::from_reader(visual_reader)?;
    let mut primitive_reader = PrimitiveReader::open(primitive_reader)?;
    let mut render_sets_data = Vec::new();

    for render_set in &visual.render_sets {

        let vertices_section = &render_set.geometry.vertices_section;
        let indices_section = &render_set.geometry.indices_section;

        let vertices = match primitive_reader.read_section::<Vertices>(vertices_section) {
            Some(Ok(v)) => v,
            Some(Err(e)) => return Err(DeError::SectionPrimitive(vertices_section.clone(), e)),
            None => return Err(DeError::MissingVerticesSection(vertices_section.clone())),
        };

        let indices = match primitive_reader.read_section::<Indices>(indices_section) {
            Some(Ok(v)) => v,
            Some(Err(e)) => return Err(DeError::SectionPrimitive(indices_section.clone(), e)),
            None => return Err(DeError::MissingIndicesSection(indices_section.clone())),
        };

        render_sets_data.push(RenderSetData {
            vertices: vertices.vertices,
            primitives: indices.primitives,
            groups: indices.groups,
        });

    }

    Ok(Model {
        visual, 
        render_sets_data,
    })

}


#[derive(Debug)]
pub struct Model {
    /// Description of the visual components of the model.
    pub visual: Box<Visual>,
    /// Decoded data for each render set.
    pub render_sets_data: Vec<RenderSetData>,
}

#[derive(Debug)]
pub struct RenderSetData {
    /// All vertices for the model. To access correct vertices,
    /// use correct method of the model to get access to them.
    pub vertices: Vec<Vertex>,
    /// Indices of the model, linking all vertices.
    pub primitives: Vec<Primitive>,
    /// Groups of indices.
    pub groups: Vec<Group>,
}

impl Model {

    /// Shortcut method to get a render set metadata together with its data.
    pub fn get_render_set(&self, index: usize) -> Option<(&RenderSet, &RenderSetData)> {
        Some((
            self.visual.render_sets.get(index)?,
            self.render_sets_data.get(index)?,
        ))
    }

}

impl RenderSetData {

    /// Get a specific primitive group. Only its vertices and primitives are
    /// returned.
    pub fn get_group(&self, index: usize) -> Option<(&[Vertex], &[Primitive])> {
        let group = self.groups.get(index)?;
        Some((
            &self.vertices[group.vertices_offset as usize..][..group.vertices_count as usize],
            &self.primitives[group.primitives_offset as usize..][..group.primitives_count as usize],
        ))
    }

}


/// Deserialization errors that can happen while read a whole compiled model.
#[derive(Debug, Error)]
pub enum DeError {
    #[error("the vertices section '{0}' is missing")]
    MissingVerticesSection(String),
    #[error("the indices section '{0}' is missing")]
    MissingIndicesSection(String),
    #[error("primitive error in section '{0}': {1}")]
    SectionPrimitive(String, primitive::DeError),
    #[error("visual error: {0}")]
    Visual(#[from] visual::DeError),
    #[error("primitive error: {0}")]
    Primitive(#[from] primitive::DeError),
}
