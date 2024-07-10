Module(
    ModModule {
        range: (),
        body: [
            Assign(
                StmtAssign {
                    range: 27..41,
                    targets: [
                        Name(
                            ExprName {
                                range: 27..31,
                                id: Identifier(
                                    "name",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 34..41,
                            value: Str(
                                "baker",
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 45..62,
                    targets: [
                        Name(
                            ExprName {
                                range: 45..52,
                                id: Identifier(
                                    "version",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 55..62,
                            value: Str(
                                "2.0.0",
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 66..89,
                    targets: [
                        Name(
                            ExprName {
                                range: 66..77,
                                id: Identifier(
                                    "description",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 80..89,
                            value: Str(
                                "UNKNOWN",
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 93..183,
                    targets: [
                        Name(
                            ExprName {
                                range: 93..101,
                                id: Identifier(
                                    "requires",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: List(
                        ExprList {
                            range: 104..183,
                            elts: [
                                Constant(
                                    ExprConstant {
                                        range: 111..123,
                                        value: Str(
                                            "qt_utils-1",
                                        ),
                                        kind: None,
                                    },
                                ),
                                Constant(
                                    ExprConstant {
                                        range: 130..138,
                                        value: Str(
                                            "appy-1",
                                        ),
                                        kind: None,
                                    },
                                ),
                                Constant(
                                    ExprConstant {
                                        range: 145..155,
                                        value: Str(
                                            "voodoo-1",
                                        ),
                                        kind: None,
                                    },
                                ),
                                Constant(
                                    ExprConstant {
                                        range: 162..180,
                                        value: Str(
                                            "parentswitcher-1",
                                        ),
                                        kind: None,
                                    },
                                ),
                            ],
                            ctx: Load,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 187..243,
                    targets: [
                        Name(
                            ExprName {
                                range: 187..195,
                                id: Identifier(
                                    "variants",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: List(
                        ExprList {
                            range: 198..243,
                            elts: [
                                List(
                                    ExprList {
                                        range: 205..219,
                                        elts: [
                                            Constant(
                                                ExprConstant {
                                                    range: 206..218,
                                                    value: Str(
                                                        "python-3.7",
                                                    ),
                                                    kind: None,
                                                },
                                            ),
                                        ],
                                        ctx: Load,
                                    },
                                ),
                                List(
                                    ExprList {
                                        range: 226..240,
                                        elts: [
                                            Constant(
                                                ExprConstant {
                                                    range: 227..239,
                                                    value: Str(
                                                        "python-3.9",
                                                    ),
                                                    kind: None,
                                                },
                                            ),
                                        ],
                                        ctx: Load,
                                    },
                                ),
                            ],
                            ctx: Load,
                        },
                    ),
                    type_comment: None,
                },
            ),
            FunctionDef(
                StmtFunctionDef {
                    range: 247..306,
                    name: Identifier(
                        "commands",
                    ),
                    args: Arguments {
                        range: (),
                        posonlyargs: [],
                        args: [],
                        vararg: None,
                        kwonlyargs: [],
                        kwarg: None,
                    },
                    body: [
                        Expr(
                            StmtExpr {
                                range: 268..306,
                                value: Call(
                                    ExprCall {
                                        range: 268..306,
                                        func: Attribute(
                                            ExprAttribute {
                                                range: 268..289,
                                                value: Attribute(
                                                    ExprAttribute {
                                                        range: 268..282,
                                                        value: Name(
                                                            ExprName {
                                                                range: 268..271,
                                                                id: Identifier(
                                                                    "env",
                                                                ),
                                                                ctx: Load,
                                                            },
                                                        ),
                                                        attr: Identifier(
                                                            "PYTHONPATH",
                                                        ),
                                                        ctx: Load,
                                                    },
                                                ),
                                                attr: Identifier(
                                                    "append",
                                                ),
                                                ctx: Load,
                                            },
                                        ),
                                        args: [
                                            Constant(
                                                ExprConstant {
                                                    range: 290..305,
                                                    value: Str(
                                                        "{root}/python",
                                                    ),
                                                    kind: None,
                                                },
                                            ),
                                        ],
                                        keywords: [],
                                    },
                                ),
                            },
                        ),
                    ],
                    decorator_list: [],
                    returns: None,
                    type_comment: None,
                    type_params: [],
                },
            ),
            Assign(
                StmtAssign {
                    range: 310..332,
                    targets: [
                        Name(
                            ExprName {
                                range: 310..319,
                                id: Identifier(
                                    "timestamp",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 322..332,
                            value: Int(
                                1712669699,
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 336..358,
                    targets: [
                        Name(
                            ExprName {
                                range: 336..351,
                                id: Identifier(
                                    "hashed_variants",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 354..358,
                            value: Bool(
                                true,
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 362..383,
                    targets: [
                        Name(
                            ExprName {
                                range: 362..376,
                                id: Identifier(
                                    "is_pure_python",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 379..383,
                            value: Bool(
                                true,
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 387..413,
                    targets: [
                        Name(
                            ExprName {
                                range: 387..395,
                                id: Identifier(
                                    "pip_name",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 398..413,
                            value: Str(
                                "baker (2.0.0)",
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 417..432,
                    targets: [
                        Name(
                            ExprName {
                                range: 417..425,
                                id: Identifier(
                                    "from_pip",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 428..432,
                            value: Bool(
                                true,
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
            Assign(
                StmtAssign {
                    range: 436..454,
                    targets: [
                        Name(
                            ExprName {
                                range: 436..450,
                                id: Identifier(
                                    "format_version",
                                ),
                                ctx: Store,
                            },
                        ),
                    ],
                    value: Constant(
                        ExprConstant {
                            range: 453..454,
                            value: Int(
                                2,
                            ),
                            kind: None,
                        },
                    ),
                    type_comment: None,
                },
            ),
        ],
        type_ignores: [],
    },
)
