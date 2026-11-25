//! The polygons of a bounds source as a tool reads them and states them again: each soup of the bound a source compiles to, in the
//! order the bound holds its polygons, and the source with one soup holding other polygons.
//!
//! [`leaves`] reads a compiled bound, so a polygon's place is the place a query over that bound names. [`Inputs::restated`] writes the
//! document and the glb again with the triangles and the other polygons one soup is to hold; every other soup states the triangles
//! it stated. Nothing is checked there: the compile of what it returns is the check, and refuses what a source cannot build.
//!
//! ⚠ A soup the compile builds again holds its polygons in another order: its triangles by material and then its other polygons, and
//! a tree bound in the order of its tree. The places of polygons after an edit are read from the bound the edit compiles to, and
//! never carried over it.

use super::*;

/// What a polygon of a soup is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolygonKind {
    Triangle,
    Sphere,
    Capsule,
    Box,
    Cylinder,
}

impl PolygonKind {
    /// How many points a polygon of this kind stands on.
    pub fn points(self) -> usize {
        match self {
            Self::Sphere => 1,
            Self::Capsule | Self::Cylinder => 2,
            Self::Triangle => 3,
            Self::Box => 4,
        }
    }

    /// The word the document calls it by.
    pub fn word(self) -> &'static str {
        match self {
            Self::Triangle => "triangle",
            Self::Sphere => "sphere",
            Self::Capsule => "capsule",
            Self::Box => "box",
            Self::Cylinder => "cylinder",
        }
    }
}

/// One polygon of a soup, in the soup's own space.
#[derive(Debug, Clone, PartialEq)]
pub struct Polygon {
    pub kind: PolygonKind,
    /// The slot of the soup's materials it names; none in a soup whose polygons name none.
    pub material: Option<u8>,
    /// The corners of a triangle, the centre of a sphere, the two ends of a capsule or a cylinder, or the four corners a box names.
    pub points: Vec<[f32; 3]>,
    /// The radius of a sphere, a capsule or a cylinder, in metres; nought for a triangle and a box.
    pub radius: f32,
}

/// What a polygon soup holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Soup {
    /// The game's value of each material, by slot.
    pub materials: Vec<u64>,
    /// The polygons, in the order the bound holds them.
    pub polygons: Vec<Polygon>,
}

/// One bound of a file that is no composite.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    /// How the document names it: `bound` at the root, `<its parent>_child<i>` under a composite.
    pub address: String,
    /// The layout it is, by the name of its declaration.
    pub kind: &'static str,
    /// The matrix of each composite it is under, the outermost first, as four columns.
    pub placed_by: Vec<[[f32; 3]; 4]>,
    /// Its polygons, for a geometry and a tree bound; none for a bound that is one shape.
    pub soup: Option<Soup>,
}

fn soup(g: &PhBoundGeometry, address: &str) -> Result<Soup, String> {
    let mut polygons = Vec::with_capacity(g.polygons.len());
    for (i, p) in g.polygons.iter().enumerate() {
        let kind = match p.primitive() {
            0 => PolygonKind::Triangle,
            1 => PolygonKind::Sphere,
            2 => PolygonKind::Capsule,
            3 => PolygonKind::Box,
            4 => PolygonKind::Cylinder,
            _ => {
                return Err(format!(
                    "{address}: polygon {i} carries a tag that names no primitive"
                ))
            }
        };
        let outside = || format!("{address}: polygon {i} names a vertex its bound does not hold");
        let (indices, radius) = g.polygon_vertices(p).ok_or_else(outside)?;
        if indices.len() != kind.points() {
            return Err(outside());
        }
        let points = indices
            .iter()
            .map(|v| g.vertex_position(*v).ok_or_else(outside))
            .collect::<Result<Vec<_>, _>>()?;
        polygons.push(Polygon {
            kind,
            material: g.poly_mat_index_list.get(i).copied(),
            points,
            radius,
        });
    }
    Ok(Soup {
        materials: g.material_ids.iter().map(|m| m.id).collect(),
        polygons,
    })
}

fn walk(
    bound: &PhBoundAny,
    address: &str,
    placed_by: &[[[f32; 3]; 4]],
    out: &mut Vec<Leaf>,
) -> Result<(), String> {
    let found = match bound {
        PhBoundAny::PhBoundComposite(composite) => {
            for (i, child) in composite.bounds.iter().enumerate() {
                let Some(child) = child.as_deref() else {
                    continue;
                };
                let m = composite.current_matrices.get(i).ok_or_else(|| {
                    format!("{address}: child {i} has no matrix, so where it stands cannot be told")
                })?;
                let mut under = placed_by.to_vec();
                under.push([xyz(&m.col0), xyz(&m.col1), xyz(&m.col2), xyz(&m.col3)]);
                walk(child, &format!("{address}_child{i}"), &under, out)?;
            }
            return Ok(());
        }
        PhBoundAny::PhBoundGeometry(g) => Some(soup(g, address)?),
        PhBoundAny::PhBoundBVH(b) => Some(soup(&b.base, address)?),
        _ => None,
    };
    out.push(Leaf {
        address: address.to_string(),
        kind: type_name(bound),
        placed_by: placed_by.to_vec(),
        soup: found,
    });
    Ok(())
}

/// The bounds of `bound` that are no composite, depth first with a null child passed over: a leaf's place in the list is the number a collider gives it.
pub fn leaves(bound: &PhBoundAny) -> Result<Vec<Leaf>, String> {
    let mut out = Vec::new();
    walk(bound, ROOT, &[], &mut out)?;
    Ok(out)
}

/// A name for the material slot `slot` of the bound at `address` that none of `names` is.
fn free_name(address: &str, slot: usize, names: &[String]) -> String {
    (slot..)
        .map(|k| material_name(address, k))
        .find(|name| !names.contains(name))
        .unwrap_or_else(|| material_name(address, slot))
}

fn prim_of(polygon: &Polygon) -> Option<Prim> {
    let point = |k: usize| polygon.points.get(k).copied();
    let radius = polygon.radius;
    let shape = match polygon.kind {
        PolygonKind::Triangle => return None,
        PolygonKind::Sphere => Shape::Sphere {
            center: point(0)?,
            radius,
        },
        PolygonKind::Capsule => Shape::Capsule {
            end0: point(0)?,
            end1: point(1)?,
            radius,
        },
        PolygonKind::Cylinder => Shape::Cylinder {
            end0: point(0)?,
            end1: point(1)?,
            radius,
        },
        PolygonKind::Box => Shape::Box {
            vertices: [point(0)?, point(1)?, point(2)?, point(3)?],
        },
    };
    Some(Prim {
        material: polygon.material,
        shape,
    })
}

/// The mesh of the soup at `address`: one primitive for each material its triangles use, named as the document names the material.
fn mesh_of(
    address: &str,
    names: &[String],
    triangles: &[Triangle],
    materials: &mut Vec<Material>,
) -> Mesh {
    let mut by_material: BTreeMap<Option<u8>, Vec<&[[f32; 3]; 3]>> = BTreeMap::new();
    for (material, corners) in triangles {
        by_material.entry(*material).or_default().push(corners);
    }
    let mut mesh = Mesh {
        name: address.to_string(),
        ..Mesh::default()
    };
    for (material, triangles) in by_material {
        let mut primitive = Primitive {
            material: material.map(|slot| {
                materials.push(Material {
                    name: names
                        .get(usize::from(slot))
                        .cloned()
                        .unwrap_or_else(|| material_name(address, usize::from(slot))),
                    ..Material::default()
                });
                materials.len() - 1
            }),
            ..Primitive::default()
        };
        let mut local: HashMap<[u32; 3], u32> = HashMap::new();
        for corner in triangles.iter().flat_map(|t| t.iter()) {
            let next = local.len() as u32;
            let index = *local.entry(bits(*corner)).or_insert(next);
            if index == next {
                primitive.positions.push(*corner);
            }
            primitive.indices.push(index);
        }
        mesh.primitives.push(primitive);
    }
    mesh
}

/// What restating a source gathers while it walks the document.
struct Restating<'a> {
    name: &'a str,
    address: &'a str,
    soup: &'a Soup,
    reading: Reading<'a>,
    meshes: Vec<Mesh>,
    materials: Vec<Material>,
    found: bool,
}

impl Restating<'_> {
    fn bound(&mut self, v: &mut Value, address: &str) -> Result<(), String> {
        let o = v
            .as_object_mut()
            .ok_or_else(|| format!("{address} is not an object"))?;
        let kind = o.get("type").and_then(Value::as_str).unwrap_or_default();
        match kind {
            "PhBoundComposite" => {
                let children = o
                    .get_mut("children")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| format!("{address} states no `children`"))?;
                for (i, child) in children.iter_mut().enumerate() {
                    match child.get_mut("bound") {
                        None | Some(Value::Null) => {}
                        Some(held) => self.bound(held, &format!("{address}_child{i}"))?,
                    }
                }
                Ok(())
            }
            "PhBoundGeometry" | "PhBoundBVH" => self.soup(o, address),
            _ => Ok(()),
        }
    }

    fn soup(&mut self, o: &mut Map<String, Value>, address: &str) -> Result<(), String> {
        let mut names: Vec<String> = o
            .get("materials")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{address} states no `materials`"))?
            .iter()
            .map(|m| {
                m.get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("{address} states a material with no `name`"))
            })
            .collect::<Result<_, _>>()?;
        let triangles: Vec<Triangle> = if address == self.address {
            self.found = true;
            let soup = self.soup;
            names.truncate(soup.materials.len());
            for slot in names.len()..soup.materials.len() {
                names.push(free_name(address, slot, &names));
            }
            let mut triangles = Vec::new();
            let mut primitives = Vec::new();
            for (i, polygon) in soup.polygons.iter().enumerate() {
                if polygon.points.len() != polygon.kind.points() {
                    return Err(format!(
                        "{address}: polygon {i} is a {} on {} points, and one stands on {}",
                        polygon.kind.word(),
                        polygon.points.len(),
                        polygon.kind.points()
                    ));
                }
                if let Some(slot) = polygon
                    .material
                    .filter(|slot| usize::from(*slot) >= names.len())
                {
                    return Err(format!(
                        "{address}: polygon {i} names material {slot} of {}",
                        names.len()
                    ));
                }
                match (prim_of(polygon), polygon.points.as_slice()) {
                    (Some(prim), _) => primitives.push(primitive_document(&prim)),
                    (None, &[a, b, c]) => triangles.push((polygon.material, [a, b, c])),
                    (None, _) => {}
                }
            }
            let materials: Vec<Value> = names
                .iter()
                .zip(&soup.materials)
                .map(|(name, id)| json!({ "name": name, "id": format!("{id:#018x}") }))
                .collect();
            o.insert("materials".into(), materials.into());
            o.insert("primitives".into(), primitives.into());
            triangles
        } else {
            match o.get("node") {
                None | Some(Value::Null) => Vec::new(),
                Some(node) => {
                    let node = node
                        .as_str()
                        .ok_or_else(|| format!("{address} states a `node` that is not a name"))?;
                    let at = Trail::default();
                    triangles_of(node, &names, &mut self.reading, &at)
                        .map_err(|fault| fault.to_string())?
                }
            }
        };
        if triangles.is_empty() {
            o.remove("node");
        } else {
            o.insert("node".into(), format!("{}_{address}", self.name).into());
            self.meshes
                .push(mesh_of(address, &names, &triangles, &mut self.materials));
        }
        Ok(())
    }
}

impl Inputs {
    /// The document of the bounds, as the source holds it.
    pub fn document(&self) -> &[u8] {
        &self.rules
    }

    /// The glb of the bounds; none for a source none of whose bounds holds a triangle.
    pub fn model(&self) -> Option<&[u8]> {
        self.model.as_deref()
    }

    /// These inputs holding `document` and `model` in the place of their own.
    pub fn with_files(&self, document: Vec<u8>, model: Option<Vec<u8>>) -> Self {
        Self {
            rules: document,
            model,
            ..self.clone()
        }
    }

    /// These inputs with the soup at `address` stating the materials and the polygons of `soup`; a material past those it stated is given a name of its own.
    pub fn restated(&self, address: &str, soup: &Soup) -> Result<Self, Vec<Problem>> {
        let scene = self
            .model
            .as_deref()
            .map(|glb| load(Path::new(MODEL), glb))
            .transpose()
            .map_err(|e| vec![problem(e.to_string(), MODEL)])?;
        let mut meshes = HashMap::new();
        for mesh in scene
            .iter()
            .flat_map(|s| &s.objects)
            .flat_map(|o| &o.meshes)
        {
            meshes.insert(strip_copy_suffix(&mesh.name).to_string(), mesh);
        }
        let mut doc = root(&self.rules).map_err(|fault| vec![problem(fault.to_string(), RULES)])?;
        let mut restating = Restating {
            name: &self.manifest.name,
            address,
            soup,
            reading: Reading {
                meshes,
                used: HashSet::new(),
                report: Vec::new(),
            },
            meshes: Vec::new(),
            materials: Vec::new(),
            found: false,
        };
        let stated = doc
            .get_mut("bound")
            .ok_or_else(|| vec![problem("states no `bound`", RULES)])?;
        restating
            .bound(stated, ROOT)
            .map_err(|why| vec![problem(why, RULES)])?;
        if !restating.found {
            return Err(vec![problem(
                format!("holds no polygon soup at `{address}`"),
                RULES,
            )]);
        }
        let name = self.manifest.name.clone();
        let model = (!restating.meshes.is_empty()).then(|| {
            let scene = Scene {
                name: name.clone(),
                objects: vec![Object {
                    name: name.clone(),
                    meshes: restating.meshes,
                    ..Object::default()
                }],
                materials: restating.materials,
                ..Scene::default()
            };
            write(&scene, &name, Container::Glb).primary
        });
        let rules = canonical(&Value::Object(doc))
            .map_err(|e| vec![problem(e.to_string(), RULES)])?
            .into_bytes();
        Ok(self.with_files(rules, model))
    }
}

/// Writes the document and the glb `inputs` hold into the bounds source in `dir`, each to a part file first; a source that holds no glb is left with none.
pub fn write_files(dir: &Path, inputs: &Inputs) -> Result<(), Vec<Problem>> {
    let part = |file: &str| dir.join(format!("{file}.part"));
    let failed = |file: &str, e: std::io::Error| {
        let _ = std::fs::remove_file(part(RULES));
        let _ = std::fs::remove_file(part(MODEL));
        vec![problem(
            format!("cannot write {}: {e}", dir.join(file).display()),
            file,
        )]
    };
    std::fs::write(part(RULES), &inputs.rules).map_err(|e| failed(RULES, e))?;
    if let Some(glb) = &inputs.model {
        std::fs::write(part(MODEL), glb).map_err(|e| failed(MODEL, e))?;
    }
    std::fs::rename(part(RULES), dir.join(RULES)).map_err(|e| failed(RULES, e))?;
    match &inputs.model {
        Some(_) => std::fs::rename(part(MODEL), dir.join(MODEL)),
        None => match std::fs::remove_file(dir.join(MODEL)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
    .map_err(|e| failed(MODEL, e))
}
