use std::path::Path;
use std::io::Write;
use std::fs::File;
use std::env;

use wgtk::model;

fn main() {

    let path_raw = env::var("WGT_MODEL_PATH").unwrap();
    let path = Path::new(&path_raw);
    let visual_file = File::open(path.with_extension("visual_processed")).unwrap();
    let primitives_file = File::open(path.with_extension("primitives_processed")).unwrap();

    let model = model::from_readers(visual_file, primitives_file).unwrap();

    // println!("{model:#?}");

    let mut obj_file = File::create("./test.obj").unwrap();
    let mut mtl_file = File::create("./test.mtl").unwrap();

    let (rs, rsd) = model.get_render_set(0).unwrap();
    let (vertices, primitives) = rsd.get_group(0).unwrap();

    let mat = &rs.geometry.primitive_groups[0].material;
    println!("properties: {:?}", mat.properties);
    println!("fx: {:?}", mat.fx);

    writeln!(obj_file, "mtllib test.mtl").unwrap();
    writeln!(obj_file, "usemtl TankMaterial").unwrap();
    for v in vertices {
        writeln!(obj_file, "v {} {} {}", v.position.x, v.position.y, v.position.z).unwrap();
    }
    for v in vertices {
        writeln!(obj_file, "vt {} {}", v.uv.x, v.uv.y).unwrap();
    }
    for v in vertices {
        writeln!(obj_file, "vn {} {} {}", v.normal.x, v.normal.y, v.normal.z).unwrap();
    }

    for p in primitives {
        writeln!(obj_file, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}", a = p.a + 1, b = p.b + 1, c = p.c + 1).unwrap();
    }

    writeln!(mtl_file, "newmtl TankMaterial").unwrap();
    writeln!(mtl_file, "map_Ka RenaultFT_hull_01_AM.png").unwrap();
    writeln!(mtl_file, "map_Kd RenaultFT_hull_01_AM.png").unwrap();
    writeln!(mtl_file, "map_Bump RenaultFT_hull_01_ANM.png").unwrap();
    writeln!(mtl_file, "map_Ks RenaultFT_hull_01_GMM.png").unwrap();


}
