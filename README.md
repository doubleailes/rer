# rer

![rer.logo](https://github.com/doubleailes/rer/blob/main/docs/content/images/res_v01.png)

**r.e.r** stand for the french sentence: "Rez En Rust"

It's a joke with the parisian sub-urban train, [réseau express régional d'Île-de-France](https://en.wikipedia.org/wiki/R%C3%A9seau_Express_R%C3%A9gional)


## Description

A rez implementation in rust

### Library

- [clap](https://github.com/clap-rs/clap) as the command line tool.
- [resolvo](https://github.com/mamba-org/resolvo) as the resolution algorithm based on CDCL SAT solving.


## Roadmap

1. Understanding the Current Codebase: We will begin by thoroughly understanding the existing "rez" codebase, including its architecture, dependencies, and overall design. This initial analysis is essential to identify potential challenges and opportunities for improvement.
2. Defining Our Goals: We need to define clear objectives for the rewrite. Is our goal to enhance performance, maintainability, or address specific issues in the original code? We will establish these objectives to guide our efforts.
3. Learning Rust: Before we begin rewriting, we will ensure that our team is well-versed in the Rust programming language. Rust has a learning curve, but it offers robust performance and safety features. We'll invest time in understanding Rust through resources like the official Rust book and practical projects.
4. Creating a Project Plan: A comprehensive project plan is crucial. We will outline the scope of the rewrite, set milestones, and establish realistic deadlines. This plan will also address how we transition from the old code to the new Rust version.
5. Start with a Prototype: To gain confidence in our Rust skills and test key concepts, we will begin with a small Rust prototype of the "rez" application. This will help us validate our approach and tooling.
6. Identify Dependencies: We will identify external dependencies used by the "rez" application and evaluate if equivalent libraries are available in Rust. If not, we may need to create Rust bindings for these dependencies.
7. Decide on the Architecture: We will make architectural decisions, considering whether we want to replicate the existing architecture or take the opportunity to improve it. Rust's ownership system may influence our design choices.
8. Migrate Incrementally: Instead of rewriting the entire codebase at once, we will opt for a gradual migration. Starting with specific modules or components, we will ensure they work seamlessly with the existing code.
9. Write Tests: Quality assurance is paramount. We will write extensive unit tests, integration tests, and potentially property-based tests. Rust has a strong testing ecosystem, and we will leverage it to maintain code quality.
10. Documenting as We Go: We'll document the code as we rewrite it. Proper documentation will enhance the understandability and maintainability of the Rust codebase.
11. Performance Profiling: We will regularly profile the application to identify bottlenecks and areas where Rust can make a difference.
12. Code Review and Collaboration: Collaboration is key. We will engage in code reviews to ensure that the code aligns with Rust best practices, is maintainable, and adheres to security standards.
13. Stay Informed About Rust Updates: Rust is an evolving language. We'll keep ourselves updated with the latest Rust releases and best practices to benefit from the latest improvements.
14. Migration Plan: As we near the completion of the Rust version, we'll formulate a migration plan. This plan will cover data migration, user transition, and other logistical aspects.
15. Testing and Validation: Thorough testing is non-negotiable. We'll ensure that the Rust version matches the functionality and quality standards of the original "rez" application.
16. Deployment: Once testing is successful, we'll deploy the Rust version to production. Rigorous monitoring will be essential to ensure performance and stability.
17. Feedback and Maintenance: We value user feedback. After deployment, we'll collect feedback and be ready to address any issues or implement improvements that may arise.
18. Retire the Old Code: When we're confident in the Rust version's stability and performance, we'll consider retiring the old "rez" codebase.

## TODO

- [ ] `rez-build`
- [ ] `rez-bind`
- [ ] `rez-env`