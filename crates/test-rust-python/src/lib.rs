use rustpython_parser::{
    ast::{Constant, StmtAssign},
    lexer::lex,
    parse_tokens, Mode,
};
use std::{collections::HashMap, error::Error, fs};

#[derive(Debug)]
#[allow(dead_code)]
pub struct Package {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    requires: Option<Vec<String>>,
    variants: Option<Vec<Vec<String>>>,
    authors: Option<Vec<String>>,
    build_requires: Option<Vec<String>>,
    cachable: Option<bool>, // If None, default value is in Config, key : default_cachable
    has_plugins: Option<bool>, // If None, default value is in Config, key : default_has_plugins
    hashed_variants: Option<bool>, // If None, default value is in Config, key : default_hashed_variants
    plugin_for: Option<String>,
    relocatable: Option<bool>, // If None, default value is in Config, key : default_relocatable
    tools: Option<Vec<String>>,
    uuid: Option<String>,
    custom_attributes: HashMap<String, Value>,
}

// https://rez.readthedocs.io/en/stable/package_definition.html#standard-package-attributes

// name: String,
// version: String,
// description: String,
// requires: Vec<String>,
// variants: Vec<String>,
// authors: Vec<String>,
// build_requires: Vec<String>,
// cachable: bool,
// has_plugins: bool,
// hashed_variants: bool,
// plugin_for: String,
// relocatable: bool,
// tools: Vec<String>,
// uuid: String,
// custom_attributes: HashMap<String, ?>,

// commands: ?
// config: ?
// help: Vec<String>,
// post_commands: ?
// pre_commands: ?
// pre_test_commands: ?
// tests: ?

impl Package {
    pub fn get_dependencies(&self) -> Vec<String> {
        self.requires.clone().unwrap_or(Vec::new())
    }
    fn from_data(raw_data: HashMap<String, Option<Value>>) -> Self {
        let mut name: Option<String> = None;
        let mut version: Option<String> = None;
        let mut description: Option<String> = None;
        let mut requires: Option<Vec<String>> = None;
        let mut variants: Option<Vec<Vec<String>>> = None;
        let mut authors: Option<Vec<String>> = None;
        let mut build_requires: Option<Vec<String>> = None;
        let mut cachable: Option<bool> = None;
        let mut has_plugins: Option<bool> = None;
        let mut hashed_variants: Option<bool> = None;
        let mut plugin_for: Option<String> = None;
        let mut relocatable: Option<bool> = None;
        let mut tools: Option<Vec<String>> = None;
        let mut uuid: Option<String> = None;

        let mut custom_attributes: HashMap<String, Value> = HashMap::new();

        for (key, value) in raw_data.iter() {
            match key.as_str() {
                "name" => {
                    if let Some(Value::Constant(Constant::Str(name_))) = value {
                        name = Some(name_.clone());
                    }
                }
                "version" => {
                    if let Some(Value::Constant(Constant::Str(version_))) = value {
                        version = Some(version_.clone());
                    }
                }
                "description" => {
                    if let Some(Value::Constant(Constant::Str(description_))) = value {
                        description = Some(description_.clone());
                    }
                }
                "requires" => {
                    if let Some(Value::List(requires_)) = value {
                        let mut requires_list: Vec<String> = Vec::new();
                        for require in requires_.iter() {
                            if let Constant::Str(require_) = require {
                                requires_list.push(require_.clone());
                            }
                        }
                        requires = Some(requires_list);
                    }
                }
                "variants" => {
                    if let Some(Value::NestedList(variants_)) = value {
                        let mut variants_list: Vec<Vec<String>> = Vec::new();
                        for variant in variants_.iter() {
                            let mut variant_list: Vec<String> = Vec::new();
                            for v in variant.iter() {
                                if let Constant::Str(v) = v {
                                    variant_list.push(v.clone());
                                }
                            }
                            variants_list.push(variant_list);
                        }
                        variants = Some(variants_list);
                    }
                }
                "authors" => {
                    if let Some(Value::List(authors_)) = value {
                        let mut authors_list: Vec<String> = Vec::new();
                        for author in authors_.iter() {
                            if let Constant::Str(author_) = author {
                                authors_list.push(author_.clone());
                            }
                        }
                        authors = Some(authors_list);
                    }
                }
                "build_requires" => {
                    if let Some(Value::List(build_requires_)) = value {
                        let mut build_requires_list: Vec<String> = Vec::new();
                        for build_require in build_requires_.iter() {
                            if let Constant::Str(build_require_) = build_require {
                                build_requires_list.push(build_require_.clone());
                            }
                        }
                        build_requires = Some(build_requires_list);
                    }
                }
                "cachable" => {
                    if let Some(Value::Constant(Constant::Bool(cachable_))) = value {
                        cachable = Some(*cachable_);
                    }
                }
                "has_plugins" => {
                    if let Some(Value::Constant(Constant::Bool(has_plugins_))) = value {
                        has_plugins = Some(*has_plugins_);
                    }
                }
                "hashed_variants" => {
                    if let Some(Value::Constant(Constant::Bool(hashed_variants_))) = value {
                        hashed_variants = Some(*hashed_variants_);
                    }
                }
                "plugin_for" => {
                    if let Some(Value::Constant(Constant::Str(plugin_for_))) = value {
                        plugin_for = Some(plugin_for_.clone());
                    }
                }
                "relocatable" => {
                    if let Some(Value::Constant(Constant::Bool(relocatable_))) = value {
                        relocatable = Some(*relocatable_);
                    }
                }
                "tools" => {
                    if let Some(Value::List(tools_)) = value {
                        let mut tools_list: Vec<String> = Vec::new();
                        for tool in tools_.iter() {
                            if let Constant::Str(tool_) = tool {
                                tools_list.push(tool_.clone());
                            }
                        }
                        tools = Some(tools_list);
                    }
                }
                "uuid" => {
                    if let Some(Value::Constant(Constant::Str(uuid_))) = value {
                        uuid = Some(uuid_.clone());
                    }
                }
                _ => {
                    custom_attributes.insert(
                        key.clone(),
                        value.clone().unwrap_or(Value::Constant(Constant::None)),
                    );
                }
            }
        }

        Package {
            name,
            version,
            description,
            requires,
            variants,
            authors,
            build_requires,
            cachable,
            has_plugins,
            hashed_variants,
            plugin_for,
            relocatable,
            tools,
            uuid,
            custom_attributes,
        }
    }

    pub fn from_file(package_path: &str) -> Result<Self, Box<dyn Error>> {
        let package = parse_package(package_path)?;
        Ok(package)
    }
}

// This enum encapsulates the different types of values that can be assigned to a variable
#[derive(Debug, Clone)]
enum Value {
    Constant(Constant),
    List(Vec<Constant>),
    NestedList(Vec<Vec<Constant>>),
}

fn parse_assign_statement(stmt: &StmtAssign) -> Option<(String, Option<Value>)> {
    // Identifier : The name of the variable being assigned
    let mut identifier: String = String::new();
    let mut value: Option<Value> = None;

    let target = stmt.targets[0].as_name_expr()?; // Not sure why multiple targets are allowed ?
    identifier.push_str(target.id.as_str());

    // Simple values encapsulated in a Constant struct
    if stmt.value.is_constant_expr() {
        let constant_expr = stmt.value.as_constant_expr()?;
        value = Some(Value::Constant(constant_expr.value.clone()));

    // List values
    } else if stmt.value.is_list_expr() {
        let list_expr = stmt.value.as_list_expr()?;
        let mut list: Vec<Constant> = Vec::new();
        let mut nested_list: Vec<Vec<Constant>> = Vec::new();
        let mut is_nested_list: bool = false;

        for expr in list_expr.elts.iter() {
            if expr.is_constant_expr() {
                let constant_expr = expr.as_constant_expr()?;
                list.push(constant_expr.value.clone());

            // Nested list values
            } else if expr.is_list_expr() {
                is_nested_list = true;
                let inner_list_expr = expr.as_list_expr()?;
                let mut inner_list: Vec<Constant> = Vec::new();
                for inner_expr in inner_list_expr.elts.iter() {
                    if inner_expr.is_constant_expr() {
                        let constant_expr = inner_expr.as_constant_expr()?;
                        inner_list.push(constant_expr.value.clone());
                    }
                }
                nested_list.push(inner_list)
            }
        }
        if is_nested_list {
            value = Some(Value::NestedList(nested_list));
        } else {
            value = Some(Value::List(list));
        }
    }

    Some((identifier, value))
}

fn parse_assign_statements(statements: Vec<&StmtAssign>) -> HashMap<String, Option<Value>> {
    let mut assign_statements: HashMap<String, Option<Value>> = HashMap::new();
    for stmt in statements {
        let (identifier, value) = match parse_assign_statement(stmt) {
            Some((identifier, value)) => (identifier, value),
            None => continue,
        };
        assign_statements.insert(identifier, value);
    }
    assign_statements
}

fn parse_package(package_path: &str) -> Result<Package, Box<dyn Error>> {
    let python_source = fs::read_to_string(package_path)?;
    let tokens = lex(python_source.as_str(), Mode::Module);
    let ast = parse_tokens(tokens, Mode::Module, "<embedded>")?;

    let assign_statements = ast
        .as_module()
        .unwrap()
        .body
        .iter()
        .filter(|stmt| stmt.is_assign_stmt())
        .map(|stmt| stmt.as_assign_stmt().unwrap())
        .collect::<Vec<&StmtAssign>>();

    let assign_statements_map = parse_assign_statements(assign_statements);

    // let function_defs = ast
    //     .as_module()
    //     .unwrap()
    //     .body
    //     .iter()
    //     .filter(|stmt| stmt.is_function_def_stmt())
    //     .map(|stmt| stmt.as_function_def_stmt().unwrap())
    //     .collect::<Vec<&StmtFunctionDef>>();

    // println!("{:#?}", function_defs);

    Ok(Package::from_data(assign_statements_map))
}
