//! Reproduce legacy v4 filtered-index fixtures using the independent 0.9.1 writers.
use base64::{engine::general_purpose::STANDARD, Engine};
use hdf5_writer::{DatasetBuilder, FilterDescription, Hdf5Builder, WriteOptions};

fn encode(builder: Hdf5Builder) -> Vec<u8> {
    builder
        .into_plan()
        .unwrap()
        .encode(WriteOptions::default())
        .unwrap()
}

fn compressed(dataset: DatasetBuilder) -> DatasetBuilder {
    dataset
        .filter(FilterDescription {
            id: 1,
            name: None,
            client_data: vec![6],
        })
        .filter(FilterDescription {
            id: 3,
            name: None,
            client_data: vec![],
        })
}

fn edge_field(maximum: Vec<u64>) -> (Vec<u8>, Vec<i32>) {
    let values: Vec<i32> = (0..297).map(|index| index * 17 - 2000).collect();
    let dataset = DatasetBuilder::typed_with_order::<i32>(
        "field",
        vec![3, 9, 11],
        hdf5_writer::ByteOrder::BigEndian,
    )
    .data(&values)
    .unwrap()
    .max_shape(maximum)
    .chunked(vec![2, 8, 8])
    .fill_value((-13i32).to_be_bytes());
    (
        Hdf5Builder::new()
            .dataset(compressed(dataset))
            .into_plan()
            .unwrap()
            .encode(WriteOptions {
                byte_order: hdf5_writer::ByteOrder::BigEndian,
                ..Default::default()
            })
            .unwrap(),
        values,
    )
}

fn strings() -> Vec<u8> {
    let text = format!("{}λ", "a".repeat(510));
    let fixed = compressed(
        DatasetBuilder::fixed_string_data("fixed", vec![1], &[&text])
            .unwrap()
            .max_shape(vec![u64::MAX])
            .chunked(vec![1]),
    );
    let variable = compressed(
        DatasetBuilder::vlen_string_data("variable", vec![1], &[&text])
            .unwrap()
            .max_shape(vec![u64::MAX])
            .chunked(vec![32]),
    );
    let bytes = encode(Hdf5Builder::new().dataset(fixed).dataset(variable));
    bytes
}

fn netcdf() -> Vec<u8> {
    use netcdf_writer::{NcAttrValue, NcFileBuilder, NcWriteFormat, NcWriteOptions};
    let mut builder = NcFileBuilder::new();
    let mut dimensions = Vec::new();
    for (name, length) in [("time", 2), ("z", 4), ("y", 8), ("x", 8)] {
        dimensions.push(builder.add_dimension(name, length).unwrap());
    }
    let variable = builder.add_variable::<i16>("density", &dimensions).unwrap();
    for (name, values) in [
        ("valid_range", vec![-60., 60.]),
        ("scale_factor", vec![0.5]),
        ("add_offset", vec![10.]),
    ] {
        builder
            .add_variable_attribute(variable, name, NcAttrValue::Doubles(values))
            .unwrap();
    }
    builder
        .set_variable_fill_value(variable, -32767i16)
        .unwrap();
    let mut stored: Vec<i16> = (0..512).map(|index| index % 180 - 90).collect();
    stored[1] = -32767;
    stored[64 + 2 * 8 + 3] = -32767;
    builder.write_variable(variable, &stored).unwrap();
    builder
        .set_variable_chunking(variable, vec![1, 4, 8, 8])
        .unwrap();
    builder
        .set_variable_deflate(variable, Some(6), true)
        .unwrap();
    let (_, bytes) = builder
        .to_vec(NcWriteOptions {
            format: NcWriteFormat::Nc4,
        })
        .unwrap();
    bytes
}

fn alembic() -> Vec<u8> {
    use hdf5_writer::{AttributeBuilder, DatasetBuilder, Hdf5Builder, WriteOptions};
    let positions: Vec<f32> = (0..171)
        .flat_map(|index| {
            [
                f32::from(i16::try_from(index - 80).unwrap()),
                f32::from(i16::try_from(index * 7 % 13).unwrap()),
                f32::from(i16::try_from(index / 11).unwrap()),
            ]
        })
        .collect();
    let text = format!("{}λ", "a".repeat(510));
    let points = DatasetBuilder::typed_data("ABC/.prop/.geom/P.smp0", vec![171, 3], &positions)
        .unwrap()
        .chunked(vec![128, 1]);
    let fixed = DatasetBuilder::fixed_string_data("ABC/.prop/fixed.smp0", vec![1], &[&text])
        .unwrap()
        .max_shape(vec![u64::MAX])
        .chunked(vec![1]);
    let variable = DatasetBuilder::vlen_string_data("ABC/.prop/variable.smp0", vec![1], &[&text])
        .unwrap()
        .max_shape(vec![u64::MAX])
        .chunked(vec![32]);
    let mut builder = Hdf5Builder::new()
        .attribute(AttributeBuilder::scalar("abc_version", -8i32).unwrap())
        .group_attribute(
            "ABC",
            AttributeBuilder::fixed_string(".prop.meta", "schema=AbcGeom_Points_v1"),
        )
        .group_attribute(
            "ABC/.prop",
            AttributeBuilder::vector(".geom.info", &[0u32]).unwrap(),
        )
        .group_attribute(
            "ABC/.prop/.geom",
            AttributeBuilder::vector("P.info", &[2u32 | (10 << 2) | (3 << 8)]).unwrap(),
        );
    for name in ["fixed", "variable"] {
        builder = builder.group_attribute(
            "ABC/.prop",
            AttributeBuilder::vector(format!("{name}.info"), &[2u32 | (12 << 2) | (1 << 8)])
                .unwrap(),
        );
    }
    for dataset in [points, fixed, variable] {
        builder = builder.dataset(hdf_compressed(dataset));
    }
    let bytes = builder
        .into_plan()
        .unwrap()
        .encode(WriteOptions::default())
        .unwrap();
    bytes
}

fn hdf_compressed(dataset: hdf5_writer::DatasetBuilder) -> hdf5_writer::DatasetBuilder {
    dataset
        .filter(hdf5_writer::FilterDescription {
            id: 1,
            name: None,
            client_data: vec![6],
        })
        .filter(hdf5_writer::FilterDescription {
            id: 3,
            name: None,
            client_data: vec![],
        })
}

/// Encode the source module used by packaged library tests.
fn source_data(output: &std::path::Path) -> String {
    let mut source = String::from(
        "//! Legacy HDF5 and NetCDF inputs retained in packaged source for regression tests.\n\n\
         // Generated by tests/fixtures/hdf5-owned/generator with its locked dependencies.\n\
         // Binary provenance and SHA-256 values are recorded alongside that generator.\n\n\
         use base64::{engine::general_purpose::STANDARD, Engine};\n\n\
         /// Decode a checked-in legacy regression payload.\n\
         ///\n\
         /// # Panics\n\
         /// Panics if a checked-in payload contains invalid base64.\n\
         pub(crate) fn decode(payload: &str) -> Vec<u8> {\n\
             STANDARD.decode(payload).expect(\"valid legacy regression payload\")\n\
         }\n",
    );
    for (file, name) in [
        ("field-fixed.h5", "FIELD_FIXED"),
        ("field-tree.h5", "FIELD_TREE"),
        ("field-extensible-source.h5", "FIELD_EXTENSIBLE_SOURCE"),
        ("strings.h5", "STRINGS"),
        ("density.nc", "DENSITY"),
        ("alembic.h5", "ALEMBIC"),
    ] {
        let encoded = STANDARD.encode(std::fs::read(output.join(file)).unwrap());
        source.push_str(&format!("\n/// Encoded `{file}`; see the fixture SHA256SUMS.\npub(crate) const {name}: &str = concat!(\n"));
        for line in encoded.as_bytes().chunks(80) {
            source.push_str(&format!(
                "    \"{}\",\n",
                std::str::from_utf8(line).unwrap()
            ));
        }
        source.push_str(");\n");
    }
    source
}

fn main() {
    let output = std::path::PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    let source = std::env::args_os().nth(2).map(std::path::PathBuf::from);
    std::fs::create_dir_all(&output).unwrap();
    for (name, maximum) in [
        ("field-fixed.h5", vec![3, 9, 11]),
        ("field-tree.h5", vec![u64::MAX, u64::MAX, 11]),
        ("field-extensible-source.h5", vec![u64::MAX, 9, 11]),
    ] {
        std::fs::write(output.join(name), edge_field(maximum).0).unwrap();
    }
    for (name, bytes) in [
        ("strings.h5", strings()),
        ("density.nc", netcdf()),
        ("alembic.h5", alembic()),
    ] {
        std::fs::write(output.join(name), bytes).unwrap();
    }
    if let Some(path) = source {
        std::fs::write(path, source_data(&output)).unwrap();
    }
}
