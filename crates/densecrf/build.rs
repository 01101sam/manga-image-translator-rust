fn main() {
    let mut build = cc::Build::new();

    build
        .cpp(true)
        .include("cpp/pydensecrf/pydensecrf/densecrf/include")
        .file("cpp/pydensecrf/pydensecrf/densecrf/src/labelcompatibility.cpp")
        .file("cpp/pydensecrf/pydensecrf/densecrf/src/pairwise.cpp")
        .file("cpp/pydensecrf/pydensecrf/densecrf/src/permutohedral.cpp")
        .file("cpp/wrapper.cpp")
        .flag_if_supported("-std=c++11")
        .compile("densecrf");

    println!("cargo:rerun-if-changed=cpp/");
}
