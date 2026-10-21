//! What the structs of an archive are.
//!
//! An archive carries no type information: a struct's type follows from the root symbol it hangs
//! off and the field that points at it, which is how the game reads it. [`types`] walks the
//! sysdolphin types (scenes, joints, meshes, materials, textures, animations, lights, fog and
//! cameras) from the root symbols it recognizes, so autolink can tell a visual asset, which should
//! come from the disc, from the structure that only organizes it.

use std::collections::VecDeque;

use crate::archive::Archive;

/// A sysdolphin struct type, named as in HSDRaw where it has an equivalent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Type {
    /// Scene data: model sets, cameras, lights and fog.
    Sobj,
    /// A model set: root joint and its joint, material and shape animations.
    JobjDesc,
    JobjDescs,
    Jobj,
    Dobj,
    Mobj,
    Material,
    PeDesc,
    Tobj,
    TobjLod,
    TobjTev,
    /// Image descriptor: size and format.
    Image,
    ImageData,
    /// Palette descriptor.
    Tlut,
    TlutData,
    Pobj,
    /// Vertex attribute descriptors of a mesh.
    Attributes,
    VertexData,
    DisplayList,
    Envelopes,
    Envelope,
    ShapeSet,
    Matrix,
    Spline,
    AnimJoints,
    AnimJoint,
    Aobj,
    AobjDesc,
    FobjDesc,
    Keyframes,
    MatAnimJoints,
    MatAnimJoint,
    MatAnim,
    TexAnim,
    ImageTable,
    TlutTable,
    ShapeAnimJoints,
    ShapeAnimJoint,
    ShapeAnim,
    /// A camera and its animations.
    CameraSet,
    Camera,
    CameraAnims,
    CameraAnim,
    Lights,
    /// A light and its animations.
    LightSet,
    Lobj,
    /// Point, spot or infinite light parameters.
    LightParams,
    LightAnims,
    LightAnim,
    /// A position in the world, such as a light's or camera's eye.
    Wobj,
    WobjAnim,
    /// Fog and its animations.
    FogSet,
    Fog,
    FogAdj,
    FogAnims,
    FogAnim,
    Name,
    /// Root of Slippi's character select file, `SlpCSSDesc` in slippi-ssbm-c.
    SlippiCss,
    SlippiChatWindow,
}

impl Type {
    pub fn name(self) -> &'static str {
        use Type::*;
        match self {
            Sobj => "sobj",
            JobjDesc => "jobj desc",
            JobjDescs => "jobj descs",
            Jobj => "jobj",
            Dobj => "dobj",
            Mobj => "mobj",
            Material => "material",
            PeDesc => "pe desc",
            Tobj => "tobj",
            TobjLod => "tobj lod",
            TobjTev => "tobj tev",
            Image => "image",
            ImageData => "image data",
            Tlut => "tlut",
            TlutData => "tlut data",
            Pobj => "pobj",
            Attributes => "attributes",
            VertexData => "vertex data",
            DisplayList => "display list",
            Envelopes => "envelopes",
            Envelope => "envelope",
            ShapeSet => "shape set",
            Matrix => "matrix",
            Spline => "spline",
            AnimJoints => "anim joints",
            AnimJoint => "anim joint",
            Aobj => "aobj",
            AobjDesc => "aobj desc",
            FobjDesc => "fobj desc",
            Keyframes => "keyframes",
            MatAnimJoints => "matanim joints",
            MatAnimJoint => "matanim joint",
            MatAnim => "matanim",
            TexAnim => "texanim",
            ImageTable => "image table",
            TlutTable => "tlut table",
            ShapeAnimJoints => "shapeanim joints",
            ShapeAnimJoint => "shapeanim joint",
            ShapeAnim => "shapeanim",
            CameraSet => "camera set",
            Camera => "camera",
            CameraAnims => "camera anims",
            CameraAnim => "camera anim",
            Lights => "lights",
            LightSet => "light set",
            Lobj => "lobj",
            LightParams => "light params",
            LightAnims => "light anims",
            LightAnim => "light anim",
            Wobj => "wobj",
            WobjAnim => "wobj anim",
            FogSet => "fog set",
            Fog => "fog",
            FogAdj => "fog adj",
            FogAnims => "fog anims",
            FogAnim => "fog anim",
            Name => "name",
            SlippiCss => "slippi css",
            SlippiChatWindow => "slippi chat window",
        }
    }

    /// Whether this type holds what something looks like: pixels, palettes, vertices, material
    /// colors, lights, fog, camera framing, placement (joints and their matrices) and motion. The
    /// rest only organizes those: descriptors, lists, render settings, draw commands, names.
    pub fn is_visual(self) -> bool {
        use Type::*;
        matches!(
            self,
            ImageData
                | TlutData
                | VertexData
                | Material
                | Jobj
                | Matrix
                | Keyframes
                | Lobj
                | LightParams
                | Wobj
                | Fog
                | FogAdj
                | Camera
        )
    }

    /// Smallest size a struct of this type can have, to catch a pointer that does not hold what
    /// its field promises.
    fn min_len(self) -> usize {
        use Type::*;
        match self {
            Jobj => 0x40,
            Tobj => 0x5c,
            Camera => 0x38,
            Matrix => 0x30,
            Lobj => 0x1c,
            Image | Pobj | Mobj | TexAnim | Attributes => 0x18,
            Dobj | JobjDesc | Aobj | MatAnim | LightAnim => 0x10,
            Material | FobjDesc | AnimJoint | Wobj | Fog => 0x14,
            Tlut => 0xe,
            PeDesc | MatAnimJoint | ShapeAnimJoint | CameraAnim => 0xc,
            ShapeAnim | AobjDesc | LightSet | WobjAnim | CameraSet | FogSet => 0x8,
            _ => 0,
        }
    }
}

fn be16(data: &[u8], at: usize) -> u16 {
    data.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

fn be32(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Type of a root symbol, by HSDRaw's naming conventions and Slippi's own roots.
pub fn root_type(name: &str) -> Option<Type> {
    let t = if name == "slpCSS" {
        Type::SlippiCss
    } else if name.ends_with("matanim_joint") {
        Type::MatAnimJoint
    } else if name.ends_with("shapeanim_joint") {
        Type::ShapeAnimJoint
    } else if name.ends_with("_animjoint") {
        Type::AnimJoint
    } else if name.ends_with("_joint") {
        Type::Jobj
    } else if name.ends_with("_camera") {
        Type::Camera
    } else if name.ends_with("_model_set") {
        Type::JobjDesc
    } else if name.ends_with("_fog") {
        Type::Fog
    } else if name.starts_with("Sc") {
        Type::Sobj
    } else {
        return None;
    };
    Some(t)
}

/// Type of what `field` of a struct of type `ty` points at, if known. `data` is the struct's
/// bytes, for fields whose meaning depends on its flags.
fn field_type(ty: Type, data: &[u8], field: u32) -> Option<Type> {
    use Type::*;
    const JOBJ_PTCL: u32 = 1 << 5;
    const JOBJ_SPLINE: u32 = 1 << 14;
    const POBJ_SHAPEANIM: u16 = 1 << 12;
    const POBJ_ENVELOPE: u16 = 1 << 13;
    let t = match (ty, field) {
        (Sobj, 0x0) => JobjDescs,
        (Sobj, 0x4) => CameraSet,
        (Sobj, 0x8) => Lights,
        (Sobj, 0xc) => FogSet,
        (JobjDescs, _) => JobjDesc,
        (JobjDesc, 0x0) => Jobj,
        (JobjDesc, 0x4) => AnimJoints,
        (JobjDesc, 0x8) => MatAnimJoints,
        (JobjDesc, 0xc) => ShapeAnimJoints,
        (Jobj, 0x0) | (Dobj, 0x0) | (Mobj, 0x0) | (Tobj, 0x0) | (Pobj, 0x0) => Name,
        (Lobj, 0x0) | (Wobj, 0x0) | (Camera, 0x0) => Name,
        (Jobj, 0x8) | (Jobj, 0xc) => Jobj,
        (Jobj, 0x10) => {
            let flags = be32(data, 0x4);
            if flags & JOBJ_SPLINE != 0 {
                Spline
            } else if flags & JOBJ_PTCL != 0 {
                return None;
            } else {
                Dobj
            }
        },
        (Jobj, 0x38) => Matrix,
        (Dobj, 0x4) => Dobj,
        (Dobj, 0x8) => Mobj,
        (Dobj, 0xc) => Pobj,
        (Mobj, 0x8) => Tobj,
        (Mobj, 0xc) => Material,
        (Mobj, 0x14) => PeDesc,
        (Tobj, 0x4) => Tobj,
        (Tobj, 0x4c) => Image,
        (Tobj, 0x50) => Tlut,
        (Tobj, 0x54) => TobjLod,
        (Tobj, 0x58) => TobjTev,
        (Image, 0x0) => ImageData,
        (Tlut, 0x0) => TlutData,
        (Pobj, 0x4) => Pobj,
        (Pobj, 0x8) => Attributes,
        (Pobj, 0x10) => DisplayList,
        (Pobj, 0x14) => {
            let flags = be16(data, 0xc);
            if flags & POBJ_SHAPEANIM != 0 {
                ShapeSet
            } else if flags & POBJ_ENVELOPE != 0 {
                Envelopes
            } else {
                Jobj
            }
        },
        (Attributes, f) if f % 0x18 == 0x14 => VertexData,
        (Envelopes, _) => Envelope,
        (Envelope, f) if f % 8 == 0 => Jobj,
        (AnimJoints, _) => AnimJoint,
        (AnimJoint, 0x0) | (AnimJoint, 0x4) => AnimJoint,
        (AnimJoint, 0x8) => Aobj,
        (Aobj, 0x8) => FobjDesc,
        (Aobj, 0xc) => Jobj,
        (FobjDesc, 0x0) => FobjDesc,
        (FobjDesc, 0x10) => Keyframes,
        (MatAnimJoints, _) => MatAnimJoint,
        (MatAnimJoint, 0x0) | (MatAnimJoint, 0x4) => MatAnimJoint,
        (MatAnimJoint, 0x8) => MatAnim,
        (MatAnim, 0x0) => MatAnim,
        (MatAnim, 0x4) => Aobj,
        (MatAnim, 0x8) => TexAnim,
        (TexAnim, 0x0) => TexAnim,
        (TexAnim, 0x8) => Aobj,
        (TexAnim, 0xc) => ImageTable,
        (TexAnim, 0x10) => TlutTable,
        (ImageTable, _) => Image,
        (TlutTable, _) => Tlut,
        (ShapeAnimJoints, _) => ShapeAnimJoint,
        (ShapeAnimJoint, 0x0) | (ShapeAnimJoint, 0x4) => ShapeAnimJoint,
        (ShapeAnimJoint, 0x8) => ShapeAnim,
        (ShapeAnim, 0x0) => ShapeAnim,
        (ShapeAnim, 0x4) => AobjDesc,
        (AobjDesc, 0x0) => AobjDesc,
        (AobjDesc, 0x4) => Aobj,
        (CameraSet, 0x0) => Camera,
        (CameraSet, 0x4) => CameraAnims,
        (Camera, 0x18) | (Camera, 0x1c) => Wobj,
        (CameraAnims, _) => CameraAnim,
        (CameraAnim, 0x0) => Aobj,
        (CameraAnim, 0x4) | (CameraAnim, 0x8) => WobjAnim,
        (Lights, _) => LightSet,
        (LightSet, 0x0) => Lobj,
        (LightSet, 0x4) => LightAnims,
        (Lobj, 0x4) => Lobj,
        (Lobj, 0x10) | (Lobj, 0x14) => Wobj,
        (Lobj, 0x18) => LightParams,
        (LightAnims, _) => LightAnim,
        (LightAnim, 0x0) => LightAnim,
        (LightAnim, 0x4) => Aobj,
        (LightAnim, 0x8) | (LightAnim, 0xc) => WobjAnim,
        (WobjAnim, 0x0) => Aobj,
        (FogSet, 0x0) => Fog,
        (FogSet, 0x4) => FogAnims,
        (Fog, 0x4) => FogAdj,
        (FogAnims, _) => FogAnim,
        (SlippiCss, 0x0) => SlippiChatWindow,
        (SlippiCss, 0x4) | (SlippiCss, 0x10) => JobjDesc,
        (SlippiCss, 0x8) => MatAnimJoint,
        (SlippiCss, 0xc) | (SlippiCss, 0x14) => Jobj,
        (SlippiChatWindow, 0x0) => Jobj,
        _ => return None,
    };
    Some(t)
}

/// The type of every extent, found by walking from the roots whose type [`root_type`] knows.
/// Extents the walk does not reach, or reaches through a field it does not know, stay `None`;
/// a struct too small for the type its field promises is left `None` and not walked further.
pub fn types(archive: &Archive) -> Vec<Option<Type>> {
    let mut types: Vec<Option<Type>> = vec![None; archive.extents.len()];
    let mut queue = VecDeque::new();
    let assign = |extent: usize, ty: Type, types: &mut Vec<Option<Type>>, queue: &mut VecDeque<usize>| {
        if types[extent].is_none() && archive.extents[extent].data.len() >= ty.min_len() {
            types[extent] = Some(ty);
            queue.push_back(extent);
        }
    };
    for root in &archive.roots {
        if let Some(ty) = root_type(&root.name) {
            assign(root.extent, ty, &mut types, &mut queue);
        }
    }
    while let Some(extent) = queue.pop_front() {
        let ty = types[extent].expect("queued extents are typed");
        let e = &archive.extents[extent];
        let mut pointers: Vec<_> = e.pointers.iter().collect();
        pointers.sort_by_key(|p| p.field);
        for p in pointers {
            if let Some(child) = field_type(ty, &e.data, p.field) {
                assign(p.target, child, &mut types, &mut queue);
            }
        }
    }
    types
}

/// Whether one struct is visual. A struct of unknown type counts as visual if it holds at least
/// `unknown_min` bytes, since below that a match with the disc is likely a coincidence.
pub fn is_visual(archive: &Archive, types: &[Option<Type>], extent: usize, unknown_min: usize) -> bool {
    match types[extent] {
        Some(ty) => ty.is_visual(),
        None => {
            let data = &archive.extents[extent].data;
            data.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1) >= unknown_min
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{Extent, Pointer, Root};

    fn extent(data: Vec<u8>, pointers: &[(u32, usize)]) -> Extent {
        Extent {
            data,
            phase: 0,
            pointers: pointers.iter().map(|&(field, target)| Pointer { field, target }).collect(),
            origin: 0,
        }
    }

    /// A joint with a mesh whose material has a texture, typed from a `_joint` root.
    #[test]
    fn types_a_textured_joint() {
        let joint = vec![0u8; 0x40];
        let mut archive = Archive::default();
        archive.extents = vec![
            extent(joint, &[(0x10, 1)]),
            extent(vec![0; 0x10], &[(0x8, 2), (0xc, 6)]),
            extent(vec![0; 0x18], &[(0x8, 3), (0xc, 7)]),
            extent(vec![0; 0x5c], &[(0x4c, 4)]),
            extent(vec![0; 0x18], &[(0x0, 5)]),
            extent(vec![9; 64], &[]),
            extent(vec![0; 0x18], &[(0x10, 8)]),
            extent(vec![1; 0x14], &[]),
            extent(vec![0x80, 0, 4, 0, 1, 2, 3], &[]),
        ];
        archive.roots = vec![Root {
            name: "Sample_joint".into(),
            extent: 0,
        }];
        let t = types(&archive);
        use Type::*;
        assert_eq!(
            t,
            [Jobj, Dobj, Mobj, Tobj, Image, ImageData, Pobj, Material, DisplayList].map(Some)
        );
        let visual: Vec<bool> = (0..t.len()).map(|i| is_visual(&archive, &t, i, 8)).collect();
        assert_eq!(visual, [true, false, false, false, false, true, false, true, false]);
    }

    /// A pointer that does not hold what its field promises is not typed, nor anything past it.
    #[test]
    fn too_small_for_its_type() {
        let mut archive = Archive::default();
        archive.extents = vec![
            extent(vec![0; 0x40], &[(0x8, 1)]),
            extent(vec![0; 0x10], &[(0x10, 2)]),
            extent(vec![0; 4], &[]),
        ];
        archive.roots = vec![Root {
            name: "Sample_joint".into(),
            extent: 0,
        }];
        assert_eq!(types(&archive), vec![Some(Type::Jobj), None, None]);
    }
}
