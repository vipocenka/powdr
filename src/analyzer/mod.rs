use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::parser;
use crate::parser::ast;
pub use crate::parser::ast::{BinaryOperator, ConstantNumberType, UnaryOperator};

pub fn analyze(path: &Path) -> Analyzed {
    let mut ctx = Context::new();
    ctx.process_file(path);
    ctx.into()
}

pub fn analyze_string(contents: &str) -> Analyzed {
    let mut ctx = Context::new();
    ctx.process_file_contents(Path::new("input"), contents);
    ctx.into()
}

#[derive(Default)]
struct Context {
    namespace: String,
    polynomial_degree: ConstantNumberType,
    /// Constants are not namespaced!
    constants: HashMap<String, ConstantNumberType>,
    definitions: HashMap<String, (Polynomial, Option<Expression>)>,
    public_declarations: HashMap<String, PublicDeclaration>,
    polynomial_identities: Vec<(Expression, SourceRef)>,
    plookups: Vec<PlookupIdentity>,
    permutations: Vec<PermutationIdentity>,
    connections: Vec<ConnectionIdentity>,
    /// The order in which definitions and identities
    /// appear in the source.
    source_order: Vec<StatementIdentifier>,
    included_files: HashSet<PathBuf>,
    current_dir: PathBuf,
    commit_poly_counter: u64,
    constant_poly_counter: u64,
    intermediate_poly_counter: u64,
    local_variables: HashMap<String, u64>,
}

pub enum StatementIdentifier {
    Definition(String),
    PublicDeclaration(String),
    Identity(usize),
    Plookup(usize),
    Permutation(usize),
    Connection(usize),
}

pub struct Analyzed {
    /// Constants are not namespaced!
    pub constants: HashMap<String, ConstantNumberType>,
    pub definitions: HashMap<String, (Polynomial, Option<Expression>)>,
    pub public_declarations: HashMap<String, PublicDeclaration>,
    pub polynomial_identities: Vec<(Expression, SourceRef)>,
    pub plookups: Vec<PlookupIdentity>,
    pub permutations: Vec<PermutationIdentity>,
    pub connections: Vec<ConnectionIdentity>,
    /// The order in which definitions and identities
    /// appear in the source.
    pub source_order: Vec<StatementIdentifier>,
}

impl Analyzed {
    /// @returns the number of committed polynomials (with multiplicities for arrays)
    pub fn commitment_count(&self) -> usize {
        self.declaration_type_count(PolynomialType::Committed)
    }
    /// @returns the number of intermediate polynomials (with multiplicities for arrays)
    pub fn intermediate_count(&self) -> usize {
        self.declaration_type_count(PolynomialType::Intermediate)
    }
    /// @returns the number of constant polynomials (with multiplicities for arrays)
    pub fn constant_count(&self) -> usize {
        self.declaration_type_count(PolynomialType::Constant)
    }

    pub fn constant_polys_in_source_order(&self) -> Vec<&(Polynomial, Option<Expression>)> {
        self.definitions_in_source_order(PolynomialType::Constant)
    }

    pub fn committed_polys_in_source_order(&self) -> Vec<&(Polynomial, Option<Expression>)> {
        self.definitions_in_source_order(PolynomialType::Committed)
    }

    pub fn definitions_in_source_order(
        &self,
        poly_type: PolynomialType,
    ) -> Vec<&(Polynomial, Option<Expression>)> {
        self.source_order
            .iter()
            .filter_map(move |statement| {
                if let StatementIdentifier::Definition(name) = statement {
                    let definition = &self.definitions[name];
                    if definition.0.poly_type == poly_type {
                        return Some(definition);
                    }
                }
                None
            })
            .collect()
    }

    fn declaration_type_count(&self, poly_type: PolynomialType) -> usize {
        self.definitions
            .iter()
            .filter_map(move |(_name, (poly, _))| {
                if poly.poly_type == poly_type {
                    Some(poly.length.unwrap_or(1) as usize)
                } else {
                    None
                }
            })
            .sum()
    }
}

impl From<Context> for Analyzed {
    fn from(
        Context {
            constants,
            definitions,
            public_declarations,
            polynomial_identities,
            plookups,
            permutations,
            connections,
            source_order,
            ..
        }: Context,
    ) -> Self {
        Self {
            constants,
            definitions,
            public_declarations,
            polynomial_identities,
            plookups,
            permutations,
            connections,
            source_order,
        }
    }
}

pub struct Polynomial {
    pub id: u64,
    pub source: SourceRef,
    pub absolute_name: String,
    pub poly_type: PolynomialType,
    pub degree: ConstantNumberType,
    pub length: Option<ConstantNumberType>,
}

impl Polynomial {
    pub fn is_array(&self) -> bool {
        self.length.is_some()
    }
}

pub struct PublicDeclaration {
    pub id: u64,
    pub source: SourceRef,
    pub name: String,
    pub polynomial: PolynomialReference,
    /// The evaluation point of the polynomial, not the array index.
    pub index: ConstantNumberType,
}

pub struct PlookupIdentity {
    pub source: SourceRef,
    pub key: SelectedExpressions,
    pub haystack: SelectedExpressions,
}

pub struct PermutationIdentity {
    pub source: SourceRef,
    pub left: SelectedExpressions,
    pub right: SelectedExpressions,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SelectedExpressions {
    pub selector: Option<Expression>,
    pub expressions: Vec<Expression>,
}

pub struct ConnectionIdentity {
    pub source: SourceRef,
    pub polynomials: Vec<Expression>,
    pub connections: Vec<Expression>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Expression {
    Constant(String),
    PolynomialReference(PolynomialReference),
    LocalVariableReference(u64),
    PublicReference(String),
    Number(ConstantNumberType),
    BinaryOperation(Box<Expression>, BinaryOperator, Box<Expression>),
    UnaryOperation(UnaryOperator, Box<Expression>),
}

#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct PolynomialReference {
    // TODO would be better to use numeric IDs instead of names,
    // but the IDs as they are overlap. Maybe we can change that.
    pub name: String,
    pub index: Option<u64>,
    pub next: bool,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum PolynomialType {
    Committed,
    Constant,
    Intermediate,
}

#[derive(Debug, Clone)]
pub struct SourceRef {
    pub file: String, // TODO should maybe be a shared pointer
    pub line: usize,
}

impl Context {
    pub fn new() -> Context {
        Context {
            namespace: "Global".to_string(),
            ..Default::default()
        }
    }

    pub fn process_file(&mut self, path: &Path) {
        let path = path
            .canonicalize()
            .unwrap_or_else(|e| panic!("File {path:?} not found: {e}"));
        if !self.included_files.insert(path.clone()) {
            return;
        }
        let contents = fs::read_to_string(path.clone()).unwrap();
        self.process_file_contents(&path, &contents);
    }

    pub fn process_file_contents(&mut self, path: &Path, contents: &str) {
        // TOOD make this work for other line endings
        let line_starts = compute_line_starts(contents);
        let pil_file =
            parser::parse(Some(path.to_str().unwrap()), contents).unwrap_or_else(|err| {
                eprintln!("Error parsing .pil file:");
                err.output_to_stderr();
                panic!();
            });
        let old_current_dir = self.current_dir.clone();
        self.current_dir = path.parent().unwrap().to_path_buf();

        let to_source_ref = |start| SourceRef {
            line: offset_to_line(start, &line_starts),
            file: path.file_name().unwrap().to_str().unwrap().to_string(),
        };

        for statement in &pil_file.0 {
            use ast::Statement;
            match statement {
                Statement::Include(_, include) => self.handle_include(include),
                Statement::Namespace(_, name, degree) => self.handle_namespace(name, degree),
                Statement::PolynomialDefinition(start, name, value) => {
                    self.handle_polynomial_definition(
                        to_source_ref(*start),
                        name,
                        &None,
                        PolynomialType::Intermediate,
                        None,
                        Some(value),
                    );
                }
                Statement::PublicDeclaration(start, name, polynomial, index) => {
                    self.handle_public_declaration(to_source_ref(*start), name, polynomial, index)
                }
                Statement::PolynomialConstantDeclaration(start, polynomials) => self
                    .handle_polynomial_declarations(
                        to_source_ref(*start),
                        polynomials,
                        PolynomialType::Constant,
                    ),
                Statement::PolynomialConstantDefinition(start, name, parameters, value) => {
                    self.handle_polynomial_definition(
                        to_source_ref(*start),
                        name,
                        &None,
                        PolynomialType::Constant,
                        Some(parameters),
                        Some(value),
                    );
                }
                Statement::PolynomialCommitDeclaration(start, polynomials) => self
                    .handle_polynomial_declarations(
                        to_source_ref(*start),
                        polynomials,
                        PolynomialType::Committed,
                    ),
                Statement::PolynomialIdentity(start, expression) => {
                    self.handle_polynomial_identity(to_source_ref(*start), expression)
                }
                Statement::PlookupIdentity(start, key, haystack) => {
                    self.handle_plookup_identity(to_source_ref(*start), key, haystack)
                }
                Statement::PermutationIdentity(start, left, right) => {
                    self.handle_permutation_identity(to_source_ref(*start), left, right)
                }
                Statement::ConnectIdentity(start, left, right) => {
                    self.handle_connect_identity(to_source_ref(*start), left, right)
                }
                Statement::ConstantDefinition(_, name, value) => {
                    self.handle_constant_definition(name, value)
                }
            }
        }

        self.current_dir = old_current_dir;
    }

    fn handle_include(&mut self, path: &str) {
        let mut dir = self.current_dir.clone();
        dir.push(path);
        self.process_file(&dir);
    }

    fn handle_namespace(&mut self, name: &str, degree: &ast::Expression) {
        self.polynomial_degree = self.evaluate_expression(degree).unwrap();
        self.namespace = name.to_owned();
    }

    fn handle_polynomial_declarations(
        &mut self,
        source: SourceRef,
        polynomials: &[ast::PolynomialName],
        polynomial_type: PolynomialType,
    ) {
        for ast::PolynomialName { name, array_size } in polynomials {
            self.handle_polynomial_definition(
                source.clone(),
                name,
                array_size,
                polynomial_type,
                None,
                None,
            );
        }
    }

    fn handle_polynomial_definition(
        &mut self,
        source: SourceRef,
        name: &String,
        array_size: &Option<ast::Expression>,
        polynomial_type: PolynomialType,
        parameters: Option<&[String]>,
        value: Option<&ast::Expression>,
    ) -> u64 {
        if parameters.is_some() {
            assert!(array_size.is_none());
            assert!(polynomial_type == PolynomialType::Constant);
        }
        let length = array_size
            .as_ref()
            .map(|l| self.evaluate_expression(l).unwrap());
        let counter = match polynomial_type {
            PolynomialType::Committed => &mut self.commit_poly_counter,
            PolynomialType::Constant => &mut self.constant_poly_counter,
            PolynomialType::Intermediate => &mut self.intermediate_poly_counter,
        };
        let id = *counter;
        *counter += length.unwrap_or(1) as u64;
        let poly = Polynomial {
            id,
            source,
            absolute_name: self.namespaced(name),
            degree: self.polynomial_degree,
            poly_type: polynomial_type,
            length,
        };
        let name = poly.absolute_name.clone();
        self.local_variables = parameters
            .map(|p| {
                p.iter()
                    .enumerate()
                    .map(|(i, p)| (p.clone(), i as u64))
                    .collect()
            })
            .unwrap_or_default();
        let value = value.map(|e| self.process_expression(e));
        self.local_variables = HashMap::default();
        let is_new = self
            .definitions
            .insert(name.clone(), (poly, value))
            .is_none();
        assert!(is_new);
        self.source_order
            .push(StatementIdentifier::Definition(name));
        id
    }

    fn handle_public_declaration(
        &mut self,
        source: SourceRef,
        name: &str,
        poly: &ast::PolynomialReference,
        index: &ast::Expression,
    ) {
        let id = self.public_declarations.len() as u64;
        self.public_declarations.insert(
            name.to_string(),
            PublicDeclaration {
                id,
                source,
                name: name.to_string(),
                polynomial: self.process_polynomial_reference(poly),
                index: self.evaluate_expression(index).unwrap(),
            },
        );
        self.source_order
            .push(StatementIdentifier::PublicDeclaration(name.to_string()));
    }

    fn handle_polynomial_identity(&mut self, source: SourceRef, expression: &ast::Expression) {
        let expr = self.process_expression(expression);
        self.polynomial_identities.push((expr, source));
        self.source_order.push(StatementIdentifier::Identity(
            self.polynomial_identities.len() - 1,
        ));
    }

    fn handle_plookup_identity(
        &mut self,
        source: SourceRef,
        key: &ast::SelectedExpressions,
        haystack: &ast::SelectedExpressions,
    ) {
        let key = self.process_selected_expression(key);
        let haystack = self.process_selected_expression(haystack);
        self.plookups.push(PlookupIdentity {
            source,
            key,
            haystack,
        });
        self.source_order
            .push(StatementIdentifier::Plookup(self.plookups.len() - 1));
    }

    fn handle_permutation_identity(
        &mut self,
        source: SourceRef,
        left: &ast::SelectedExpressions,
        right: &ast::SelectedExpressions,
    ) {
        let left = self.process_selected_expression(left);
        let right = self.process_selected_expression(right);
        self.permutations.push(PermutationIdentity {
            source,
            left,
            right,
        });
        self.source_order.push(StatementIdentifier::Permutation(
            self.permutations.len() - 1,
        ));
    }

    fn handle_connect_identity(
        &mut self,
        source: SourceRef,

        left: &[ast::Expression],
        right: &[ast::Expression],
    ) {
        self.connections.push(ConnectionIdentity {
            source,
            polynomials: self.process_expressions(left),
            connections: self.process_expressions(right),
        });
        self.source_order
            .push(StatementIdentifier::Connection(self.connections.len() - 1));
    }

    fn handle_constant_definition(&mut self, name: &str, value: &ast::Expression) {
        // TODO does the order matter here?
        let is_new = self
            .constants
            .insert(name.to_string(), self.evaluate_expression(value).unwrap())
            .is_none();
        assert!(is_new, "Constant {name} was defined twice.");
    }

    fn namespaced(&self, name: &String) -> String {
        self.namespaced_ref(&None, name)
    }

    fn namespaced_ref(&self, namespace: &Option<String>, name: &String) -> String {
        format!("{}.{name}", namespace.as_ref().unwrap_or(&self.namespace))
    }

    fn process_selected_expression(&self, expr: &ast::SelectedExpressions) -> SelectedExpressions {
        SelectedExpressions {
            selector: expr.selector.as_ref().map(|e| self.process_expression(e)),
            expressions: self.process_expressions(&expr.expressions),
        }
    }

    fn process_expressions(&self, exprs: &[ast::Expression]) -> Vec<Expression> {
        exprs.iter().map(|e| self.process_expression(e)).collect()
    }

    fn process_expression(&self, expr: &ast::Expression) -> Expression {
        match expr {
            ast::Expression::Constant(name) => Expression::Constant(name.clone()),
            ast::Expression::PolynomialReference(poly) => {
                if poly.namespace.is_none() && self.local_variables.contains_key(&poly.name) {
                    let id = self.local_variables[&poly.name];
                    assert!(!poly.next);
                    assert!(poly.index.is_none());
                    Expression::LocalVariableReference(id)
                } else {
                    Expression::PolynomialReference(self.process_polynomial_reference(poly))
                }
            }
            ast::Expression::PublicReference(name) => Expression::PublicReference(name.clone()),
            ast::Expression::Number(n) => Expression::Number(*n),
            ast::Expression::BinaryOperation(left, op, right) => {
                if let Some(value) = self.evaluate_binary_operation(left, op, right) {
                    Expression::Number(value)
                } else {
                    Expression::BinaryOperation(
                        Box::new(self.process_expression(left)),
                        *op,
                        Box::new(self.process_expression(right)),
                    )
                }
            }
            ast::Expression::UnaryOperation(op, value) => {
                if let Some(value) = self.evaluate_unary_operation(op, value) {
                    Expression::Number(value)
                } else {
                    Expression::UnaryOperation(*op, Box::new(self.process_expression(value)))
                }
            }
        }
    }

    fn process_polynomial_reference(&self, poly: &ast::PolynomialReference) -> PolynomialReference {
        let index = poly
            .index
            .as_ref()
            .map(|i| self.evaluate_expression(i).unwrap() as u64);
        PolynomialReference {
            name: self.namespaced_ref(&poly.namespace, &poly.name),
            index,
            next: poly.next,
        }
    }

    fn evaluate_expression(&self, expr: &ast::Expression) -> Option<ConstantNumberType> {
        match expr {
            ast::Expression::Constant(name) => Some(
                *self
                    .constants
                    .get(name)
                    .unwrap_or_else(|| panic!("Constant {name} not found.")),
            ),
            ast::Expression::PolynomialReference(_) => None,
            ast::Expression::PublicReference(_) => None,
            ast::Expression::Number(n) => Some(*n),
            ast::Expression::BinaryOperation(left, op, right) => {
                self.evaluate_binary_operation(left, op, right)
            }
            ast::Expression::UnaryOperation(op, value) => self.evaluate_unary_operation(op, value),
        }
    }

    fn evaluate_binary_operation(
        &self,
        left: &ast::Expression,
        op: &BinaryOperator,
        right: &ast::Expression,
    ) -> Option<ConstantNumberType> {
        // TODO handle owerflow and maybe use bigint instead.
        if let (Some(left), Some(right)) = (
            self.evaluate_expression(left),
            self.evaluate_expression(right),
        ) {
            Some(match op {
                BinaryOperator::Add => left + right,
                BinaryOperator::Sub => left - right,
                BinaryOperator::Mul => left * right,
                BinaryOperator::Div => left / right,
                BinaryOperator::Pow => {
                    assert!(right <= u32::MAX.into());
                    left.pow(right as u32)
                }
            })
        } else {
            None
        }
    }

    fn evaluate_unary_operation(
        &self,
        op: &UnaryOperator,
        value: &ast::Expression,
    ) -> Option<ConstantNumberType> {
        // TODO handle owerflow and maybe use bigint instead.
        self.evaluate_expression(value).map(|v| match op {
            UnaryOperator::Plus => v,
            UnaryOperator::Minus => -v,
        })
    }
}

fn compute_line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect::<Vec<_>>()
}

fn offset_to_line(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(line) => line + 1,
        Err(next_line) => next_line,
    }
}

#[cfg(test)]
mod test {
    use super::{compute_line_starts, offset_to_line};

    #[test]
    pub fn line_calc() {
        let input = "abc\nde";
        let breaks = compute_line_starts(input);
        let lines = (0..input.len())
            .map(|o| offset_to_line(o, &breaks))
            .collect::<Vec<_>>();
        assert_eq!(lines, [1, 1, 1, 1, 2, 2]);
    }

    #[test]
    pub fn line_calc_empty_start() {
        let input = "\nab\n\nc\nde\n";
        let breaks = compute_line_starts(input);
        let lines = (0..input.len())
            .map(|o| offset_to_line(o, &breaks))
            .collect::<Vec<_>>();
        assert_eq!(lines, [1, 2, 2, 2, 3, 4, 4, 5, 5, 5]);
    }
}
