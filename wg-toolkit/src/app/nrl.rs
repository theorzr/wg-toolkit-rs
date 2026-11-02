//! Client-side state of the NRL node tree (`BW::NRL`), the layer replicating dynamic
//! components, enough to decode updates to its nodes.
//!
//! The `Nrl*` elements in [`super::client::element`] carry the messages, but an update
//! only names a node id: what that node is (which component, which of its properties or
//! methods) was said when it was created, possibly elements earlier. [`NrlTree`] keeps
//! that, along with the property values needed to decode nested changes and the buffer
//! that `NrlData` fragments are reassembled in.

use std::collections::HashMap;
use std::sync::Arc;
use std::io;

use crate::util::io::WgReadExt;
use crate::net::codec::Codec;
use crate::script::{Ty, TyKind, Value};

use super::client::element::{
    decode_compressed_path, EntityPropertyPathStep, NrlBody, NrlChild, NrlNode, NrlStream, PathOutcome,
};
use super::dispatch::{EntityDispatch, ScriptDispatch};


/// What the client knows of the NRL tree, see the module documentation.
#[derive(Debug, Default)]
pub struct NrlTree {
    nodes: HashMap<u32, Node>,
    /// The `NrlData` fragments not yet consumed by an update.
    buffer: Vec<u8>,
}

#[derive(Debug)]
struct Node {
    parent: Option<u32>,
    kind: NodeKind,
}

#[derive(Debug)]
enum NodeKind {
    Entity { entity_id: u32 },
    Component,
    /// A component's property, by index in its property table, with its tracked value.
    Property { type_id: u16, index: usize, value: Option<Value> },
    /// A component's method child, by index in its client method table when known: a
    /// child not marked present at creation carries no index.
    Method { type_id: u16, index: Option<usize> },
    /// Any other node: the root, a component's trailing child, or one this tree cannot
    /// place.
    Other,
}

/// One decoded update to a node, see [`NrlTree::update_node`].
#[derive(Debug, Clone)]
pub struct NrlUpdate {
    pub node_id: u16,
    /// The entity owning the updated node's component, when the tree saw it created.
    pub entity_id: Option<u32>,
    /// The component's own type id, see `ScriptDispatch::component_from_nrl_id`.
    pub type_id: u16,
    pub change: NrlChange,
}

/// What an [`NrlUpdate`] changes.
#[derive(Debug, Clone)]
pub enum NrlChange {
    /// Kind 0: the property's whole new value.
    Set { property: Arc<str>, value: Value },
    /// Kind 2: one value inside a property, at a compressed path. The path starts at the
    /// component's property table, exactly as `NestedEntityProperty`'s starts at an
    /// entity's, so `property` is the one it names and `path` the rest of it.
    Nested { property: Arc<str>, path: Vec<EntityPropertyPathStep>, value: Value },
    /// Kind 1: a range `start..end` of a list inside a property replaced by `values`, at a
    /// compressed path like [`Self::Nested`]'s (see `SliceEntityProperty`).
    Slice { property: Arc<str>, path: Vec<EntityPropertyPathStep>, start: u32, end: u32, values: Vec<Value> },
    /// A property change of a kind not decoded yet. Its length is on the wire, so the
    /// records after it still decode.
    Unknown { property: Arc<str>, kind: u8, data: Vec<u8> },
    /// A call of one of the component's client methods.
    Call { method: Arc<str>, args: Vec<Value> },
}

impl NrlTree {

    pub fn new() -> Self {
        Self::default()
    }

    /// Forget every node and any buffered data, as on a new connection.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.buffer.clear();
    }

    /// Register the nodes a stream creates and drop the ones it unlinks. Components are
    /// only placed once decoded, see [`NrlStream::decode_components`].
    pub fn apply(&mut self, stream: &NrlStream, dispatch: &ScriptDispatch) {
        for message in &stream.messages {
            let node_id = u32::from(message.node_id);
            match &message.body {
                NrlBody::CreateNode { parent_id, node } =>
                    self.create(node_id, Some(u32::from(*parent_id)), node, dispatch),
                NrlBody::CreateChildren { nodes, .. } => {
                    for (child_id, node) in nodes {
                        self.create(*child_id, Some(node_id), node, dispatch);
                    }
                }
                NrlBody::UnlinkTree => self.unlink(node_id),
                NrlBody::Lookup | NrlBody::UpdateNode | NrlBody::Other { .. } => {}
            }
        }
    }

    fn create(&mut self, node_id: u32, parent: Option<u32>, node: &NrlNode, dispatch: &ScriptDispatch) {
        let kind = match node {
            NrlNode::Entity { entity_id } => NodeKind::Entity { entity_id: *entity_id },
            NrlNode::DynComponent { children, data: Some(data), .. } => {
                // Every child exists, present or not: a child left out at creation just
                // has nothing to say yet, and can be updated later.
                let present: HashMap<u32, &NrlChild> = data.children.iter()
                    .map(|(id, child)| (*id, child))
                    .collect();
                let properties = dispatch.entity_from_id(data.type_id)
                    .map_or(0, |component| component.properties.len());
                for (index, &child_id) in children.ids.iter().enumerate() {
                    let kind = match present.get(&child_id) {
                        Some(NrlChild::Property { value, .. }) =>
                            NodeKind::Property { type_id: data.type_id, index, value: Some(value.clone()) },
                        Some(NrlChild::Method { method_index, .. }) =>
                            NodeKind::Method { type_id: data.type_id, index: usize::try_from(*method_index).ok() },
                        Some(NrlChild::Trailing) => NodeKind::Other,
                        // A property child's place says which property it is, present or
                        // not, and an absent one holds its default value: lists start empty
                        // and fill through slice changes. A method child's place says
                        // nothing: only its index names its method.
                        None if index < properties => NodeKind::Property {
                            type_id: data.type_id,
                            index,
                            value: dispatch.entity_from_id(data.type_id)
                                .and_then(|component| component.properties.get(index))
                                .map(|property| default_value(&property.ty)),
                        },
                        None if index + 1 < children.ids.len() =>
                            NodeKind::Method { type_id: data.type_id, index: None },
                        None => NodeKind::Other,
                    };
                    self.nodes.insert(child_id, Node { parent: Some(node_id), kind });
                }
                NodeKind::Component
            }
            _ => NodeKind::Other,
        };
        self.nodes.insert(node_id, Node { parent, kind });
    }

    /// Remove a node and everything under it.
    fn unlink(&mut self, node_id: u32) {
        let mut removed = vec![node_id];
        while let Some(id) = removed.pop() {
            self.nodes.remove(&id);
            removed.extend(self.nodes.iter()
                .filter(|(_, node)| node.parent == Some(id))
                .map(|(&child_id, _)| child_id));
        }
    }

    /// Append an `NrlData` fragment to the buffer updates are read from.
    pub fn push_data(&mut self, fragment: &[u8]) {
        self.buffer.extend_from_slice(fragment);
    }

    /// Consume one record from the buffer, for an `NrlUpdateNodeFlag`: `[node_id: u16
    /// big-endian]` and that node's update. On error the buffer is dropped, since a
    /// record of unknown length leaves nothing after it readable.
    pub fn update_node(&mut self, dispatch: &ScriptDispatch) -> io::Result<NrlUpdate> {
        let buffer = std::mem::take(&mut self.buffer);
        let mut read = &buffer[..];
        let node_id = read_node_id(&mut read)?;
        let update = self.decode_update(dispatch, node_id, &mut read).map_err(|e| {
            let shown = &buffer[..buffer.len().min(48)];
            invalid(format!("node {node_id}: {e} (buffer: {shown:02X?}, {} bytes)", buffer.len()))
        })?;
        self.buffer = read.to_vec();
        Ok(update)
    }

    /// Consume one record from the buffer, for an `NrlUnlinkTreeFlag`: just the node id.
    pub fn unlink_tree_node(&mut self) -> io::Result<u16> {
        let mut read = &self.buffer[..];
        let node_id = read_node_id(&mut read)?;
        self.buffer.drain(..2);
        self.unlink(u32::from(node_id));
        Ok(node_id)
    }

    /// Decode an update to `node_id` from `read`, as the buffer holds it after the node
    /// id, and apply it to the tracked values.
    pub fn decode_update(&mut self, dispatch: &ScriptDispatch, node_id: u16, read: &mut &[u8]) -> io::Result<NrlUpdate> {

        let key = u32::from(node_id);
        let entity_id = self.entity_of(key);
        let node = self.nodes.get(&key)
            .ok_or_else(|| invalid(format!("update to unknown node {node_id}")))?;
        let parent = node.parent;

        let (type_id, change) = match node.kind {
            NodeKind::Property { type_id, index, .. } => {

                let component = dispatch.entity_from_id(type_id)
                    .ok_or_else(|| invalid(format!("unknown component type id {type_id}")))?;
                let property = component.properties.get(index)
                    .ok_or_else(|| invalid(format!("component {type_id} has no property {index}")))?;

                // A volatile property's update is its whole value alone.
                if property.volatile {
                    let new = Value::read(read, &property.ty)?;
                    if let Some(NodeKind::Property { value, .. }) = self.nodes.get_mut(&key).map(|node| &mut node.kind) {
                        *value = Some(new.clone());
                    }
                    return Ok(NrlUpdate { node_id, entity_id, type_id, change: NrlChange::Set { property: Arc::clone(&property.name), value: new } });
                }

                let kind = read.read_u8()?;
                let len = read.read_packed_u24()? as usize;
                if read.len() < len {
                    return Err(invalid(format!("node {node_id}: {len} bytes announced, {} left", read.len())));
                }
                let (data, rest) = read.split_at(len);
                *read = rest;

                let change = match kind {
                    0 => {
                        let mut data_read = data;
                        let new = Value::read(&mut data_read, &property.ty)?;
                        if !data_read.is_empty() {
                            return Err(invalid(format!("node {node_id}: {} bytes left after the value", data_read.len())));
                        }
                        if let Some(NodeKind::Property { value, .. }) = self.nodes.get_mut(&key).map(|node| &mut node.kind) {
                            *value = Some(new.clone());
                        }
                        NrlChange::Set { property: Arc::clone(&property.name), value: new }
                    }
                    1 => self.decode_nested(component, type_id, parent, data, true)?,
                    2 => self.decode_nested(component, type_id, parent, data, false)?,
                    _ => NrlChange::Unknown { property: Arc::clone(&property.name), kind, data: data.to_vec() },
                };

                (type_id, change)

            }
            NodeKind::Method { type_id, index } => {
                let component = dispatch.entity_from_id(type_id)
                    .ok_or_else(|| invalid(format!("unknown component type id {type_id}")))?;
                let method = index.and_then(|index| component.client_methods.get(index))
                    .ok_or_else(|| invalid(format!("node {node_id}: method child with no known method")))?;
                let args = method.read_args(&mut *read)?;
                (type_id, NrlChange::Call { method: Arc::clone(&method.name), args })
            }
            _ => return Err(invalid(format!("update to node {node_id}, neither a property nor a method"))),
        };

        Ok(NrlUpdate { node_id, entity_id, type_id, change })

    }

    /// Decode a nested or slice change, whose path starts at the component's property
    /// table and so is decoded against every property of the component, then apply it.
    fn decode_nested(
        &mut self,
        component: &EntityDispatch,
        type_id: u16,
        parent: Option<u32>,
        data: &[u8],
        is_slice: bool,
    ) -> io::Result<NrlChange> {

        // The tracked values are moved out for the decode, and back after it, rather than
        // cloned: some are whole battle rosters, changed thousands of times a battle.
        let siblings: Vec<(u32, Arc<str>)> = self.nodes.iter()
            .filter_map(|(&id, node)| match node.kind {
                NodeKind::Property { type_id: sibling_type_id, index, .. }
                    if node.parent == parent && sibling_type_id == type_id =>
                    Some((id, Arc::clone(&component.properties.get(index)?.name))),
                _ => None,
            })
            .collect();
        let mut values = HashMap::new();
        for (id, name) in &siblings {
            if let Some(NodeKind::Property { value: Some(value), .. }) = self.nodes.get_mut(id).map(|node| &mut node.kind) {
                values.insert(Arc::clone(name), std::mem::replace(value, Value::None));
            }
        }

        let outcome = decode_compressed_path(&component.properties, Some(&values), data, is_slice);
        match &outcome {
            Ok(PathOutcome::Single { path, value }) => {
                if let Some(slot) = value_at_path_in(&mut values, path) {
                    *slot = value.clone();
                }
            }
            Ok(PathOutcome::Slice { path, start, end, values: new }) => {
                if let Some(Value::Seq(items)) = value_at_path_in(&mut values, path) {
                    let (start, end) = (*start as usize, *end as usize);
                    if start <= end && end <= items.len() {
                        items.splice(start..end, new.iter().cloned());
                    }
                }
            }
            Err(_) => {}
        }

        for (id, name) in &siblings {
            if let Some(value) = values.remove(name) {
                if let Some(NodeKind::Property { value: slot, .. }) = self.nodes.get_mut(id).map(|node| &mut node.kind) {
                    *slot = Some(value);
                }
            }
        }

        // A path from a property table always starts with the property it names.
        let property = |path: &mut Vec<EntityPropertyPathStep>| match path.remove(0) {
            EntityPropertyPathStep::Field(property) => property,
            EntityPropertyPathStep::Index(_) => unreachable!("a path from a property table starts with a property"),
        };
        Ok(match outcome? {
            PathOutcome::Single { mut path, value } =>
                NrlChange::Nested { property: property(&mut path), path, value },
            PathOutcome::Slice { mut path, start, end, values } =>
                NrlChange::Slice { property: property(&mut path), path, start, end, values },
        })

    }

    /// The entity a node belongs to, by walking up its parents.
    fn entity_of(&self, mut node_id: u32) -> Option<u32> {
        for _ in 0..16 {
            let node = self.nodes.get(&node_id)?;
            if let NodeKind::Entity { entity_id } = node.kind {
                return Some(entity_id);
            }
            node_id = node.parent?;
        }
        None
    }

}

/// The value a property of type `ty` holds before anything set it: zeros, empty lists,
/// and `None` for a dict that allows it. Strings and pickles, never crossed by a path,
/// are left `None`.
fn default_value(ty: &Ty) -> Value {
    match ty.kind() {
        TyKind::Alias(inner) => default_value(inner),
        TyKind::Int8 => Value::Int8(0),
        TyKind::Int16 => Value::Int16(0),
        TyKind::Int32 => Value::Int32(0),
        TyKind::Int64 => Value::Int64(0),
        TyKind::UInt8 => Value::UInt8(0),
        TyKind::UInt16 => Value::UInt16(0),
        TyKind::UInt32 => Value::UInt32(0),
        TyKind::UInt64 => Value::UInt64(0),
        TyKind::Float32 => Value::Float32(0.0),
        TyKind::Float64 => Value::Float64(0.0),
        TyKind::Vector2 => Value::Vector2(Default::default()),
        TyKind::Vector3 => Value::Vector3(Default::default()),
        TyKind::Vector4 => Value::Vector4(Default::default()),
        TyKind::Dict(dict) if dict.allow_none => Value::None,
        TyKind::Dict(dict) => Value::Dict(dict.properties.iter()
            .map(|prop| (Arc::clone(&prop.name), default_value(&prop.ty)))
            .collect()),
        TyKind::Array(_) => Value::Seq(Vec::new()),
        TyKind::Tuple(seq) => Value::Seq((0..seq.size.unwrap_or(0)).map(|_| default_value(&seq.ty)).collect()),
        TyKind::String | TyKind::Python | TyKind::Mailbox => Value::None,
    }
}

/// The value at `path`, whose first step names a property of `values`.
fn value_at_path_in<'v>(values: &'v mut HashMap<Arc<str>, Value>, path: &[EntityPropertyPathStep]) -> Option<&'v mut Value> {
    let (EntityPropertyPathStep::Field(name), rest) = path.split_first()? else { return None };
    value_at_path(values.get_mut(name)?, rest)
}

/// The value at `path` inside `value`.
fn value_at_path<'v>(value: &'v mut Value, path: &[EntityPropertyPathStep]) -> Option<&'v mut Value> {
    let Some((step, rest)) = path.split_first() else { return Some(value) };
    let child = match (value, step) {
        (Value::Dict(dict), EntityPropertyPathStep::Field(name)) => dict.get_mut(name)?,
        (Value::Seq(items), EntityPropertyPathStep::Index(index)) => items.get_mut(*index as usize)?,
        _ => return None,
    };
    value_at_path(child, rest)
}

/// A node id, which NRL writes big-endian unlike everything else.
fn read_node_id(read: &mut &[u8]) -> io::Result<u16> {
    let mut bytes = [0; 2];
    io::Read::read_exact(read, &mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
