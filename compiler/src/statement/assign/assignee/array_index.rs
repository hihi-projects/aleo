// Copyright (C) 2019-2021 Aleo Systems Inc.
// This file is part of the Leo library.

// The Leo library is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The Leo library is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with the Leo library. If not, see <https://www.gnu.org/licenses/>.

//! Resolves assignees in a compiled Leo program.

use std::convert::TryInto;

use crate::{
    errors::{ExpressionError, IntegerError, StatementError},
    program::ConstrainedProgram,
    value::ConstrainedValue,
    GroupType,
    Integer,
};
use leo_asg::{ConstInt, Expression, Node};

use snarkvm_fields::PrimeField;
use snarkvm_gadgets::{eq::EvaluateEqGadget, select::CondSelectGadget};
use snarkvm_r1cs::ConstraintSystem;

use super::ResolverContext;

impl<'a, F: PrimeField, G: GroupType<F>> ConstrainedProgram<'a, F, G> {
    pub(super) fn resolve_target_access_array_index<'b, CS: ConstraintSystem<F>>(
        &mut self,
        cs: &mut CS,
        mut context: ResolverContext<'a, 'b, F, G>,
        index: &'a Expression<'a>,
    ) -> Result<(), StatementError> {
        println!("RTAAI input len {}", context.input.len());
        let input_len = context.input.len();
        if input_len == 0 {
            return Err(StatementError::array_assign_interior_index(&context.span));
        }

        let index_resolved = self.enforce_index(cs, index, &context.span)?;
        match (context.input.remove(0), input_len) {
            (ConstrainedValue::Array(input), 1) => {
                if let Some(index) = index_resolved.to_usize() {
                    if index >= input.len() {
                        Err(StatementError::array_assign_index_bounds(
                            index,
                            input.len(),
                            &context.span,
                        ))
                    } else {
                        let target = input.get_mut(index).unwrap();
                        if context.remaining_accesses.is_empty() {
                            self.enforce_assign_context(cs, &context, target)
                        } else {
                            context.input = vec![target];
                            self.resolve_target_access(cs, context)
                        }
                    }
                } else {
                    let span = index.span().cloned().unwrap_or_default();
                    {
                        let array_len: u32 = input
                            .len()
                            .try_into()
                            .map_err(|_| ExpressionError::array_length_out_of_bounds(&span))?;
                        self.array_bounds_check(cs, &&index_resolved, array_len, &span)?;
                    }

                    for (i, item) in input.iter_mut().enumerate() {
                        let namespace_string = format!(
                            "evaluate dyn array assignment eq {} {}:{}",
                            i, span.line_start, span.col_start
                        );
                        let eq_namespace = cs.ns(|| namespace_string);

                        let index_bounded = i
                            .try_into()
                            .map_err(|_| ExpressionError::array_index_out_of_legal_bounds(&span))?;
                        let const_index = ConstInt::U32(index_bounded).cast_to(&index_resolved.get_type());
                        let index_comparison = index_resolved
                            .evaluate_equal(eq_namespace, &Integer::new(&const_index))
                            .map_err(|_| ExpressionError::cannot_evaluate("==".to_string(), &span))?;

                        let mut unique_namespace = cs.ns(|| {
                            format!(
                                "select array dyn assignment {} {}:{}",
                                i, span.line_start, span.col_start
                            )
                        });
                        let temp_item = {
                            let mut item = item.clone();
                            let mut new_context = ResolverContext {
                                input: vec![&mut item],
                                span: context.span.clone(),
                                target_value: context.target_value.clone(),
                                remaining_accesses: context.remaining_accesses,
                                indicator: context.indicator,
                                operation: context.operation,
                            };
                            if context.remaining_accesses.is_empty() {
                                let item = new_context.input.remove(0);
                                self.enforce_assign_context(&mut unique_namespace, &new_context, item)?;
                            } else {
                                self.resolve_target_access(&mut unique_namespace, new_context)?;
                            }
                            item
                        };
                        let value = ConstrainedValue::conditionally_select(
                            unique_namespace,
                            &index_comparison,
                            &temp_item,
                            &item,
                        )
                        .map_err(|e| ExpressionError::cannot_enforce("conditional select".to_string(), e, &span))?;
                        *item = value;
                    }
                    Ok(())
                }
            }
            (target, _) => {
                // index of array range
                if let Some(index) = index_resolved.to_usize() {
                    if index >= input_len {
                        return Err(StatementError::array_assign_index_bounds(
                            index,
                            input_len,
                            &context.span,
                        ));
                    }
                    if context.remaining_accesses.is_empty() {
                        self.enforce_assign_context(cs, &context, target)
                    } else {
                        context.input = vec![target];
                        self.resolve_target_access(cs, context)
                    }
                } else {
                    let span = index.span().cloned().unwrap_or_default();

                    Err(StatementError::from(IntegerError::invalid_integer(
                        index_resolved.to_string(),
                        &span,
                    )))
                }
            }
        }
    }
}
