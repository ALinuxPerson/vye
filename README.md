# Emyu: The Viewless M~~V~~U Framework

_It's MVU, but we lost the view along the way..._

> **⚠️ IMPORTANT**: This project is under heavy construction and prototyping! Expect breaking changes and incomplete 
> features.

# Overview

**Emyu** is a framework for building business logic in Rust using the Model-View-Update (MVU) architecture pattern, but
where the View is implemented in a foreign language, such as Dart/Flutter, Java/Kotlin/Android, Swift/iOS, and others.

# Example

```rust
pub type MyApp = AdHocApp<MyRootModel>;

pub struct MyRootModel {
    name: Signal<String>,
    age: Signal<i32>,
    employed: bool,
}

#[emyu::model(for_app = "MyApp", dispatcher(meta(base(derive(Clone)))))]
pub impl MyRootModel {
    pub fn new();

    pub fn set_attributes(&mut self, name: String, age: i32, employed: bool) {
        self.name.writer().set(name);
        self.age.writer().set(age);
        self.employed = employed;
    }

    pub fn name(&self) -> Signal<String>;
    pub fn age(&self) -> Signal<i32>;
}
```

# License

Licensed under either of:

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in these crate(s) by you,
as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.