//! Text DMX (`keyvalues2`): the model source format Source 2 tools write. The explorer shows a
//! summary of what a file holds (models, meshes with vertex and face counts, materials, the
//! joint tree) above its text. Binary DMX is recognised and reported, not decoded.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

/// Elements past this count stop the parse, so a hostile file cannot run a worker out of memory.
const MAX_ELEMENTS: usize = 500_000;
/// Nesting past this depth is refused.
const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Attr {
    /// A scalar of any type, as written (`"1 0 0"` for a vector).
    Value { kind: String, value: String },
    /// An array of scalars.
    Values { kind: String, values: Vec<String> },
    /// One element: inline (its index) or by id.
    Element(Link),
    /// An array of elements.
    Elements(Vec<Link>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    Inline(usize),
    Id(String),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub kind: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub attrs: Vec<(String, Attr)>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, a)| a)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The header: `<!-- dmx encoding keyvalues2 1 format model 22 -->`.
    pub encoding: String,
    pub format: String,
    pub elements: Vec<Element>,
    /// Top-level element indexes.
    pub roots: Vec<usize>,
}

impl Document {
    fn resolve(&self, link: &Link, ids: &HashMap<&str, usize>) -> Option<usize> {
        match link {
            Link::Inline(index) => Some(*index),
            Link::Id(id) => ids.get(id.as_str()).copied(),
            Link::Null => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshSummary {
    pub name: String,
    pub vertices: usize,
    pub faces: usize,
    pub materials: Vec<String>,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub encoding: String,
    pub format: String,
    pub elements: usize,
    pub types: Vec<(String, usize)>,
    pub meshes: Vec<MeshSummary>,
    /// Joint names, each indented by its depth in the skeleton.
    pub joints: Vec<(usize, String)>,
    pub up_axis: Option<String>,
}

/// True for a binary DMX header.
pub fn is_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(64)];
    String::from_utf8_lossy(head).starts_with("<!-- dmx encoding binary")
}

/// Parse a text DMX. The error says why a file is not one this reads.
pub fn parse(text: &str) -> Result<Document, String> {
    let text = text.trim_start_matches('\u{feff}');
    let header_end = text
        .find("-->")
        .ok_or_else(|| "No DMX header line.".to_string())?;
    let header = &text[..header_end];
    let words: Vec<&str> = header.split_whitespace().collect();
    // <!-- dmx encoding keyvalues2 1 format model 22
    if words.len() < 4 || words[1] != "dmx" || words[2] != "encoding" {
        return Err("No DMX header line.".to_string());
    }
    if words[3] != "keyvalues2" && words[3] != "keyvalues2_flat" {
        return Err(format!("This is a {} DMX. Only text (keyvalues2) DMX is read.", words[3]));
    }
    let format = words
        .iter()
        .position(|w| *w == "format")
        .and_then(|at| words.get(at + 1..at + 3))
        .map(|parts| parts.join(" "))
        .unwrap_or_default();
    let mut parser = Parser {
        tokens: tokenize(&text[header_end + 3..])?,
        at: 0,
        elements: Vec::new(),
    };
    let mut roots = Vec::new();
    while parser.peek().is_some() {
        let kind = parser.string()?;
        roots.push(parser.element(kind, 0)?);
    }
    Ok(Document {
        encoding: words[3..5.min(words.len())].join(" "),
        format,
        elements: parser.elements,
        roots,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
    OpenList,
    CloseList,
    Comma,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '[' => tokens.push(Token::OpenList),
            ']' => tokens.push(Token::CloseList),
            ',' => tokens.push(Token::Comma),
            '"' => {
                let mut value = String::new();
                loop {
                    match chars.next() {
                        Some('\\') => match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some(other) => value.push(other),
                            None => return Err("A string ends in an escape.".to_string()),
                        },
                        Some('"') => break,
                        Some(other) => value.push(other),
                        None => return Err("A string is not closed.".to_string()),
                    }
                }
                tokens.push(Token::Str(value));
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            other => {
                // A bare word (some writers leave type names unquoted).
                let mut value = String::from(other);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || "{}[],\"".contains(next) {
                        break;
                    }
                    value.push(next);
                    chars.next();
                }
                tokens.push(Token::Str(value));
            }
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    elements: Vec<Element>,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn string(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Token::Str(value)) => Ok(value),
            other => Err(format!("Expected a string, found {other:?}.")),
        }
    }

    fn expect(&mut self, token: Token) -> Result<(), String> {
        match self.next() {
            Some(found) if found == token => Ok(()),
            other => Err(format!("Expected {token:?}, found {other:?}.")),
        }
    }

    /// After the type name: `{ attributes }`. Returns the element's index.
    fn element(&mut self, kind: String, depth: usize) -> Result<usize, String> {
        if depth > MAX_DEPTH {
            return Err("Elements nest too deep.".to_string());
        }
        if self.elements.len() >= MAX_ELEMENTS {
            return Err("Too many elements.".to_string());
        }
        self.expect(Token::Open)?;
        let index = self.elements.len();
        self.elements.push(Element {
            kind,
            id: None,
            name: None,
            attrs: Vec::new(),
        });
        let mut attrs = Vec::new();
        let mut id = None;
        let mut name = None;
        loop {
            match self.peek() {
                Some(Token::Close) => {
                    self.at += 1;
                    break;
                }
                None => return Err("An element is not closed.".to_string()),
                _ => {}
            }
            let attr_name = self.string()?;
            let attr_kind = self.string()?;
            // An inline element: "name" "DmeType" { ... }
            if self.peek() == Some(&Token::Open) {
                let child = self.element(attr_kind, depth + 1)?;
                attrs.push((attr_name, Attr::Element(Link::Inline(child))));
                continue;
            }
            let attr = if attr_kind == "element_array" {
                Attr::Elements(self.element_list(depth)?)
            } else if attr_kind.ends_with("_array") {
                Attr::Values {
                    kind: attr_kind.trim_end_matches("_array").to_string(),
                    values: self.value_list()?,
                }
            } else if attr_kind == "element" {
                let value = self.string()?;
                Attr::Element(if value.is_empty() { Link::Null } else { Link::Id(value) })
            } else {
                let value = self.string()?;
                if attr_name == "id" && attr_kind == "elementid" {
                    id = Some(value.clone());
                }
                if attr_name == "name" && attr_kind == "string" {
                    name = Some(value.clone());
                }
                Attr::Value {
                    kind: attr_kind,
                    value,
                }
            };
            attrs.push((attr_name, attr));
        }
        let element = &mut self.elements[index];
        element.attrs = attrs;
        element.id = id;
        element.name = name;
        Ok(index)
    }

    fn value_list(&mut self) -> Result<Vec<String>, String> {
        self.expect(Token::OpenList)?;
        let mut values = Vec::new();
        loop {
            match self.next() {
                Some(Token::CloseList) => return Ok(values),
                Some(Token::Comma) => {}
                Some(Token::Str(value)) => values.push(value),
                other => return Err(format!("Unexpected {other:?} in an array.")),
            }
        }
    }

    fn element_list(&mut self, depth: usize) -> Result<Vec<Link>, String> {
        self.expect(Token::OpenList)?;
        let mut links = Vec::new();
        loop {
            match self.next() {
                Some(Token::CloseList) => return Ok(links),
                Some(Token::Comma) => {}
                Some(Token::Str(value)) => {
                    if self.peek() == Some(&Token::Open) {
                        links.push(Link::Inline(self.element(value, depth + 1)?));
                    } else if value == "element" {
                        // "element" "id" form.
                        let id = self.string()?;
                        links.push(if id.is_empty() { Link::Null } else { Link::Id(id) });
                    } else {
                        links.push(Link::Id(value));
                    }
                }
                other => return Err(format!("Unexpected {other:?} in an element array.")),
            }
        }
    }
}

/// What a DMX holds: the counts by element type, each mesh, the joint tree and the up axis.
pub fn summarize(document: &Document) -> Summary {
    let ids: HashMap<&str, usize> = document
        .elements
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.id.as_deref().map(|id| (id, i)))
        .collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for element in &document.elements {
        *counts.entry(element.kind.as_str()).or_default() += 1;
    }
    let mut types: Vec<(String, usize)> = counts.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    types.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let meshes = document
        .elements
        .iter()
        .filter(|e| e.kind == "DmeMesh")
        .map(|mesh| mesh_summary(document, mesh, &ids))
        .collect();

    let mut joints = Vec::new();
    let mut seen = vec![false; document.elements.len()];
    for model in document.elements.iter().filter(|e| e.kind == "DmeModel") {
        walk_joints(document, model, 0, &ids, &mut seen, &mut joints);
    }

    let up_axis = document
        .elements
        .iter()
        .find(|e| e.kind == "DmeAxisSystem")
        .and_then(|axis| match axis.attr("upAxis") {
            Some(Attr::Value { value, .. }) => Some(
                match value.as_str() {
                    "1" => "X",
                    "2" => "Y",
                    "3" => "Z",
                    other => other,
                }
                .to_string(),
            ),
            _ => None,
        });

    Summary {
        encoding: document.encoding.clone(),
        format: document.format.clone(),
        elements: document.elements.len(),
        types,
        meshes,
        joints,
        up_axis,
    }
}

fn mesh_summary(document: &Document, mesh: &Element, ids: &HashMap<&str, usize>) -> MeshSummary {
    let state = ["bindState", "currentState"]
        .iter()
        .find_map(|name| match mesh.attr(name) {
            Some(Attr::Element(link)) => document.resolve(link, ids),
            _ => None,
        })
        .or_else(|| match mesh.attr("baseStates") {
            Some(Attr::Elements(links)) => links.first().and_then(|l| document.resolve(l, ids)),
            _ => None,
        });
    let vertices = state
        .map(|index| &document.elements[index])
        .and_then(|data| {
            ["position$0", "positions"]
                .iter()
                .find_map(|name| match data.attr(name) {
                    Some(Attr::Values { values, .. }) => Some(values.len()),
                    _ => None,
                })
        })
        .unwrap_or(0);
    let mut faces = 0;
    let mut materials = Vec::new();
    if let Some(Attr::Elements(sets)) = mesh.attr("faceSets") {
        for link in sets {
            let Some(set) = document.resolve(link, ids).map(|i| &document.elements[i]) else {
                continue;
            };
            if let Some(Attr::Values { values, .. }) = set.attr("faces") {
                faces += values.iter().filter(|v| v.trim() == "-1").count();
            }
            if let Some(Attr::Element(link)) = set.attr("material")
                && let Some(material) = document.resolve(link, ids).map(|i| &document.elements[i])
                && let Some(Attr::Value { value, .. }) = material.attr("mtlName")
                && !materials.contains(value)
            {
                materials.push(value.clone());
            }
        }
    }
    let visible = !matches!(mesh.attr("visible"), Some(Attr::Value { value, .. }) if value == "0" || value == "false");
    MeshSummary {
        name: mesh.name.clone().unwrap_or_default(),
        vertices,
        faces,
        materials,
        visible,
    }
}

fn walk_joints(
    document: &Document,
    node: &Element,
    depth: usize,
    ids: &HashMap<&str, usize>,
    seen: &mut [bool],
    out: &mut Vec<(usize, String)>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(Attr::Elements(children)) = node.attr("children") else {
        return;
    };
    for link in children {
        let Some(index) = document.resolve(link, ids) else {
            continue;
        };
        if seen[index] {
            continue;
        }
        seen[index] = true;
        let child = &document.elements[index];
        let next = if child.kind == "DmeJoint" {
            out.push((depth, child.name.clone().unwrap_or_default()));
            depth + 1
        } else {
            depth
        };
        walk_joints(document, child, next, ids, seen, out);
    }
}

/// The summary as the lines the preview shows above the text.
pub fn summary_text(document: &Document) -> String {
    let summary = summarize(document);
    let mut out = String::new();
    out.push_str(&format!(
        "// {} DMX, format {}, {} elements",
        summary.encoding, summary.format, summary.elements
    ));
    if let Some(up) = &summary.up_axis {
        out.push_str(&format!(", up axis {up}"));
    }
    out.push('\n');
    if summary.meshes.is_empty() {
        out.push_str("// No meshes: skeleton only.\n");
    }
    for mesh in &summary.meshes {
        out.push_str(&format!(
            "// Mesh {}: {} vertices, {} faces{}{}\n",
            mesh.name,
            mesh.vertices,
            mesh.faces,
            if mesh.materials.is_empty() {
                String::new()
            } else {
                format!(", materials {}", mesh.materials.join(", "))
            },
            if mesh.visible { "" } else { " (hidden)" }
        ));
    }
    if !summary.joints.is_empty() {
        out.push_str(&format!("// Skeleton: {} joints\n", summary.joints.len()));
        for (depth, name) in summary.joints.iter().take(64) {
            out.push_str("//   ");
            out.push_str(&"  ".repeat(*depth));
            out.push_str(name);
            out.push('\n');
        }
        if summary.joints.len() > 64 {
            out.push_str(&format!("//   and {} more\n", summary.joints.len() - 64));
        }
    }
    out
}
