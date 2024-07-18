use anyhow::Result;
use sqlite3_parser::ast::{self, Expr, UnaryOperator};

use crate::{
    function::{Func, SingleRowFunc},
    schema::{Column, Schema, Table},
    select::{ColumnInfo, Select, SrcTable},
    util::normalize_ident,
    vdbe::{BranchOffset, Insn, ProgramBuilder},
};

pub fn build_select<'a>(schema: &Schema, select: &'a ast::Select) -> Result<Select<'a>> {
    match &select.body.select {
        ast::OneSelect::Select {
            columns,
            from: Some(from),
            where_clause,
            ..
        } => {
            let (table_name, maybe_alias) = match &from.select {
                Some(select_table) => match select_table.as_ref() {
                    ast::SelectTable::Table(name, alias, ..) => (
                        &name.name,
                        alias.as_ref().map(|als| match als {
                            ast::As::As(alias) => alias,     // users as u
                            ast::As::Elided(alias) => alias, // users u
                        }),
                    ),
                    _ => todo!(),
                },
                None => todo!(),
            };
            let table_name = &table_name.0;
            let maybe_alias = maybe_alias.map(|als| &als.0);
            let table = match schema.get_table(table_name) {
                Some(table) => table,
                None => anyhow::bail!("Parse error: no such table: {}", table_name),
            };
            let mut joins = Vec::new();
            joins.push(SrcTable {
                table: Table::BTree(table.clone()),
                alias: maybe_alias,
                join_info: None,
            });
            if let Some(selected_joins) = &from.joins {
                for join in selected_joins {
                    let (table_name, maybe_alias) = match &join.table {
                        ast::SelectTable::Table(name, alias, ..) => (
                            &name.name,
                            alias.as_ref().map(|als| match als {
                                ast::As::As(alias) => alias,     // users as u
                                ast::As::Elided(alias) => alias, // users u
                            }),
                        ),
                        _ => todo!(),
                    };
                    let table_name = &table_name.0;
                    let maybe_alias = maybe_alias.as_ref().map(|als| &als.0);
                    let table = match schema.get_table(table_name) {
                        Some(table) => table,
                        None => anyhow::bail!("Parse error: no such table: {}", table_name),
                    };
                    joins.push(SrcTable {
                        table: Table::BTree(table),
                        alias: maybe_alias,
                        join_info: Some(join),
                    });
                }
            }

            let _table = Table::BTree(table);
            let column_info = analyze_columns(columns, &joins);
            let exist_aggregation = column_info
                .iter()
                .any(|info| info.is_aggregation_function());
            Ok(Select {
                columns,
                column_info,
                src_tables: joins,
                limit: &select.limit,
                exist_aggregation,
                where_clause,
                loops: Vec::new(),
            })
        }
        ast::OneSelect::Select {
            columns,
            from: None,
            where_clause,
            ..
        } => {
            let column_info = analyze_columns(columns, &Vec::new());
            let exist_aggregation = column_info
                .iter()
                .any(|info| info.is_aggregation_function());
            Ok(Select {
                columns,
                column_info,
                src_tables: Vec::new(),
                limit: &select.limit,
                where_clause,
                exist_aggregation,
                loops: Vec::new(),
            })
        }
        _ => todo!(),
    }
}

pub fn translate_expr(
    program: &mut ProgramBuilder,
    select: &Select,
    expr: &ast::Expr,
    target_register: usize,
) -> Result<usize> {
    match expr {
        ast::Expr::Between { .. } => todo!(),
        ast::Expr::Binary(e1, op, e2) => {
            let e1_reg = program.alloc_register();
            let e2_reg = program.alloc_register();
            let _ = translate_expr(program, select, e1, e1_reg)?;
            let _ = translate_expr(program, select, e2, e2_reg)?;

            match op {
                ast::Operator::NotEquals => {
                    let if_true_label = program.allocate_label();
                    wrap_eval_jump_expr(
                        program,
                        Insn::Ne {
                            lhs: e1_reg,
                            rhs: e2_reg,
                            target_pc: if_true_label,
                        },
                        target_register,
                        if_true_label,
                    );
                }
                ast::Operator::Equals => {
                    let if_true_label = program.allocate_label();
                    wrap_eval_jump_expr(
                        program,
                        Insn::Eq {
                            lhs: e1_reg,
                            rhs: e2_reg,
                            target_pc: if_true_label,
                        },
                        target_register,
                        if_true_label,
                    );
                }
                ast::Operator::Less => {
                    let if_true_label = program.allocate_label();
                    wrap_eval_jump_expr(
                        program,
                        Insn::Lt {
                            lhs: e1_reg,
                            rhs: e2_reg,
                            target_pc: if_true_label,
                        },
                        target_register,
                        if_true_label,
                    );
                }
                ast::Operator::LessEquals => {
                    let if_true_label = program.allocate_label();
                    wrap_eval_jump_expr(
                        program,
                        Insn::Le {
                            lhs: e1_reg,
                            rhs: e2_reg,
                            target_pc: if_true_label,
                        },
                        target_register,
                        if_true_label,
                    );
                }
                ast::Operator::Greater => {
                    let if_true_label = program.allocate_label();
                    wrap_eval_jump_expr(
                        program,
                        Insn::Gt {
                            lhs: e1_reg,
                            rhs: e2_reg,
                            target_pc: if_true_label,
                        },
                        target_register,
                        if_true_label,
                    );
                }
                ast::Operator::GreaterEquals => {
                    let if_true_label = program.allocate_label();
                    wrap_eval_jump_expr(
                        program,
                        Insn::Ge {
                            lhs: e1_reg,
                            rhs: e2_reg,
                            target_pc: if_true_label,
                        },
                        target_register,
                        if_true_label,
                    );
                }
                ast::Operator::Add => {
                    program.emit_insn(Insn::Add {
                        lhs: e1_reg,
                        rhs: e2_reg,
                        dest: target_register,
                    });
                }
                other_unimplemented => todo!("{:?}", other_unimplemented),
            }
            Ok(target_register)
        }
        ast::Expr::Case { .. } => todo!(),
        ast::Expr::Cast { .. } => todo!(),
        ast::Expr::Collate(_, _) => todo!(),
        ast::Expr::DoublyQualified(_, _, _) => todo!(),
        ast::Expr::Exists(_) => todo!(),
        ast::Expr::FunctionCall {
            name,
            distinctness: _,
            args,
            filter_over: _,
        } => {
            let func_type: Option<Func> = match normalize_ident(name.0.as_str()).as_str().parse() {
                Ok(func) => Some(func),
                Err(_) => None,
            };
            match func_type {
                Some(Func::Agg(_)) => {
                    anyhow::bail!("Parse error: aggregation function in non-aggregation context")
                }
                Some(Func::SingleRow(srf)) => {
                    match srf {
                        SingleRowFunc::Coalesce => {
                            let args = if let Some(args) = args {
                                if args.len() < 2 {
                                    anyhow::bail!(
                                        "Parse error: coalesce function with less than 2 arguments"
                                    );
                                }
                                args
                            } else {
                                anyhow::bail!("Parse error: coalesce function with no arguments");
                            };

                            // coalesce function is implemented as a series of not null checks
                            // whenever a not null check succeeds, we jump to the end of the series
                            let label_coalesce_end = program.allocate_label();
                            for (index, arg) in args.iter().enumerate() {
                                let reg = translate_expr(program, select, arg, target_register)?;
                                if index < args.len() - 1 {
                                    program.emit_insn_with_label_dependency(
                                        Insn::NotNull {
                                            reg,
                                            target_pc: label_coalesce_end,
                                        },
                                        label_coalesce_end,
                                    );
                                }
                            }
                            program.preassign_label_to_next_insn(label_coalesce_end);

                            Ok(target_register)
                        }
                        SingleRowFunc::Like => {
                            let args = if let Some(args) = args {
                                if args.len() < 2 {
                                    anyhow::bail!(
                                        "Parse error: like function with less than 2 arguments"
                                    );
                                }
                                args
                            } else {
                                anyhow::bail!("Parse error: like function with no arguments");
                            };
                            for arg in args {
                                let reg = program.alloc_register();
                                let _ = translate_expr(program, select, arg, reg)?;
                                match arg {
                                    ast::Expr::Literal(_) => program.mark_last_insn_constant(),
                                    _ => {}
                                }
                            }
                            program.emit_insn(Insn::Function {
                                start_reg: target_register + 1,
                                dest: target_register,
                                func: SingleRowFunc::Like,
                            });
                            Ok(target_register)
                        }
                        SingleRowFunc::Abs => {
                            let args = if let Some(args) = args {
                                if args.len() != 1 {
                                    anyhow::bail!(
                                        "Parse error: abs function with not exactly 1 argument"
                                    );
                                }
                                args
                            } else {
                                anyhow::bail!("Parse error: abs function with no arguments");
                            };

                            let regs = program.alloc_register();
                            let _ = translate_expr(program, select, &args[0], regs)?;
                            program.emit_insn(Insn::Function {
                                start_reg: regs,
                                dest: target_register,
                                func: SingleRowFunc::Abs,
                            });

                            Ok(target_register)
                        }
                    }
                }
                None => {
                    anyhow::bail!("Parse error: unknown function {}", name.0);
                }
            }
        }
        ast::Expr::FunctionCallStar { .. } => todo!(),
        ast::Expr::Id(ident) => {
            // let (idx, col) = table.unwrap().get_column(&ident.0).unwrap();
            let (idx, col, cursor_id) = resolve_ident_table(program, &ident.0, select)?;
            if col.primary_key {
                program.emit_insn(Insn::RowId {
                    cursor_id,
                    dest: target_register,
                });
            } else {
                program.emit_insn(Insn::Column {
                    column: idx,
                    dest: target_register,
                    cursor_id,
                });
            }
            maybe_apply_affinity(col, target_register, program);
            Ok(target_register)
        }
        ast::Expr::InList { .. } => todo!(),
        ast::Expr::InSelect { .. } => todo!(),
        ast::Expr::InTable { .. } => todo!(),
        ast::Expr::IsNull(_) => todo!(),
        ast::Expr::Like { .. } => todo!(),
        ast::Expr::Literal(lit) => match lit {
            ast::Literal::Numeric(val) => {
                let maybe_int = val.parse::<i64>();
                if let Ok(int_value) = maybe_int {
                    program.emit_insn(Insn::Integer {
                        value: int_value,
                        dest: target_register,
                    });
                } else {
                    // must be a float
                    program.emit_insn(Insn::Real {
                        value: val.parse().unwrap(),
                        dest: target_register,
                    });
                }
                Ok(target_register)
            }
            ast::Literal::String(s) => {
                program.emit_insn(Insn::String8 {
                    value: s[1..s.len() - 1].to_string(),
                    dest: target_register,
                });
                Ok(target_register)
            }
            ast::Literal::Blob(_) => todo!(),
            ast::Literal::Keyword(_) => todo!(),
            ast::Literal::Null => {
                program.emit_insn(Insn::Null {
                    dest: target_register,
                });
                Ok(target_register)
            }
            ast::Literal::CurrentDate => todo!(),
            ast::Literal::CurrentTime => todo!(),
            ast::Literal::CurrentTimestamp => todo!(),
        },
        ast::Expr::Name(_) => todo!(),
        ast::Expr::NotNull(_) => todo!(),
        ast::Expr::Parenthesized(_) => todo!(),
        ast::Expr::Qualified(tbl, ident) => {
            let (idx, col, cursor_id) = resolve_ident_qualified(program, &tbl.0, &ident.0, select)?;
            if col.primary_key {
                program.emit_insn(Insn::RowId {
                    cursor_id,
                    dest: target_register,
                });
            } else {
                program.emit_insn(Insn::Column {
                    column: idx,
                    dest: target_register,
                    cursor_id,
                });
            }
            maybe_apply_affinity(col, target_register, program);
            Ok(target_register)
        }
        ast::Expr::Raise(_, _) => todo!(),
        ast::Expr::Subquery(_) => todo!(),
        ast::Expr::Unary(op, expr) => match (op, expr.as_ref()) {
            (UnaryOperator::Negative, ast::Expr::Literal(ast::Literal::Numeric(numeric_value))) => {
                let maybe_int = numeric_value.parse::<i64>();
                if let Ok(value) = maybe_int {
                    program.emit_insn(Insn::Integer {
                        value: -value,
                        dest: target_register,
                    });
                } else {
                    program.emit_insn(Insn::Real {
                        value: -numeric_value.parse::<f64>()?,
                        dest: target_register,
                    });
                }
                Ok(target_register)
            }
            _ => todo!(),
        },
        ast::Expr::Variable(_) => todo!(),
    }
}

pub fn analyze_columns<'a>(
    columns: &'a Vec<ast::ResultColumn>,
    joins: &Vec<SrcTable>,
) -> Vec<ColumnInfo<'a>> {
    let mut column_information_list = Vec::with_capacity(columns.len());
    for column in columns {
        let mut info = ColumnInfo::new();
        if let ast::ResultColumn::Star = column {
            info.columns_to_allocate = 0;
            for join in joins {
                info.columns_to_allocate += join.table.columns().len();
            }
        } else {
            info.columns_to_allocate = 1;
            analyze_column(column, &mut info);
        }
        column_information_list.push(info);
    }
    column_information_list
}

/// Analyze a column expression.
///
/// This function will walk all columns and find information about:
/// * Aggregation functions.
fn analyze_column<'a>(column: &'a ast::ResultColumn, column_info_out: &mut ColumnInfo<'a>) {
    match column {
        ast::ResultColumn::Expr(expr, _) => analyze_expr(expr, column_info_out),
        ast::ResultColumn::Star => {}
        ast::ResultColumn::TableStar(_) => {}
    }
}

pub fn analyze_expr<'a>(expr: &'a Expr, column_info_out: &mut ColumnInfo<'a>) {
    match expr {
        ast::Expr::FunctionCall {
            name,
            distinctness: _,
            args,
            filter_over: _,
        } => {
            let func_type = match normalize_ident(name.0.as_str()).as_str().parse() {
                Ok(func) => Some(func),
                Err(_) => None,
            };
            if func_type.is_none() {
                let args = args.as_ref().unwrap();
                if !args.is_empty() {
                    analyze_expr(args.first().unwrap(), column_info_out);
                }
            } else {
                column_info_out.func = func_type;
                // TODO(pere): use lifetimes for args? Arenas would be lovely here :(
                column_info_out.args = args;
            }
        }
        ast::Expr::FunctionCallStar { .. } => todo!(),
        _ => {}
    }
}

fn wrap_eval_jump_expr(
    program: &mut ProgramBuilder,
    insn: Insn,
    target_register: usize,
    if_true_label: BranchOffset,
) {
    program.emit_insn(Insn::Integer {
        value: 1, // emit True by default
        dest: target_register,
    });
    program.emit_insn_with_label_dependency(insn, if_true_label);
    program.emit_insn(Insn::Integer {
        value: 0, // emit False if we reach this point (no jump)
        dest: target_register,
    });
    program.preassign_label_to_next_insn(if_true_label);
}

pub fn resolve_ident_qualified<'a>(
    program: &ProgramBuilder,
    table_name: &String,
    ident: &String,
    select: &'a Select,
) -> Result<(usize, &'a Column, usize)> {
    for join in &select.src_tables {
        match join.table {
            Table::BTree(ref table) => {
                let table_identifier = match join.alias {
                    Some(alias) => alias.clone(),
                    None => table.name.to_string(),
                };
                if table_identifier == *table_name {
                    let res = table
                        .columns
                        .iter()
                        .enumerate()
                        .find(|(_, col)| col.name == *ident);
                    if res.is_some() {
                        let (idx, col) = res.unwrap();
                        let cursor_id = program.resolve_cursor_id(&table_identifier);
                        return Ok((idx, col, cursor_id));
                    }
                }
            }
            Table::Pseudo(_) => todo!(),
        }
    }
    anyhow::bail!(
        "Parse error: column with qualified name {}.{} not found",
        table_name,
        ident
    );
}

pub fn resolve_ident_table<'a>(
    program: &ProgramBuilder,
    ident: &String,
    select: &'a Select,
) -> Result<(usize, &'a Column, usize)> {
    let mut found = Vec::new();
    for join in &select.src_tables {
        match join.table {
            Table::BTree(ref table) => {
                let table_identifier = match join.alias {
                    Some(alias) => alias.clone(),
                    None => table.name.to_string(),
                };
                let res = table
                    .columns
                    .iter()
                    .enumerate()
                    .find(|(_, col)| col.name == *ident);
                if res.is_some() {
                    let (idx, col) = res.unwrap();
                    let cursor_id = program.resolve_cursor_id(&table_identifier);
                    found.push((idx, col, cursor_id));
                }
            }
            Table::Pseudo(_) => todo!(),
        }
    }
    if found.len() == 1 {
        return Ok(found[0]);
    }
    if found.is_empty() {
        anyhow::bail!("Parse error: column with name {} not found", ident.as_str());
    }

    anyhow::bail!("Parse error: ambiguous column name {}", ident.as_str());
}

pub fn maybe_apply_affinity(col: &Column, target_register: usize, program: &mut ProgramBuilder) {
    if col.ty == crate::schema::Type::Real {
        program.emit_insn(Insn::RealAffinity {
            register: target_register,
        })
    }
}