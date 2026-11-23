import * as Blockly from 'blockly';

/**
 * Lume 积木定义 / 工具箱 / 生成器。
 * 语法依据：README.md「Lume 语法速览」、docs/lume_spec.md、docs/math-and-logging.md。
 * 生成结果使用缩进块 + end，不使用花括号与分号。
 */

const TYPES = [
  ['int', 'int'],
  ['float', 'float'],
  ['bool', 'bool'],
  ['string', 'string'],
  ['i16', 'i16'],
  ['pointer', 'pointer'],
];

const FN1 = [
  ['print', 'print'],
  ['len', 'len'],
  ['sin', 'sin'],
  ['cos', 'cos'],
  ['tan', 'tan'],
  ['asin', 'asin'],
  ['acos', 'acos'],
  ['atan', 'atan'],
  ['sinh', 'sinh'],
  ['cosh', 'cosh'],
  ['tanh', 'tanh'],
  ['exp', 'exp'],
  ['exp2', 'exp2'],
  ['expm1', 'expm1'],
  ['ln', 'ln'],
  ['log', 'log'],
  ['log2', 'log2'],
  ['log10', 'log10'],
  ['log1p', 'log1p'],
  ['sqrt', 'sqrt'],
  ['cbrt', 'cbrt'],
  ['floor', 'floor'],
  ['ceil', 'ceil'],
  ['round', 'round'],
  ['trunc', 'trunc'],
  ['abs', 'abs'],
  ['trace', 'trace'],
  ['debug', 'debug'],
  ['info', 'info'],
  ['warn', 'warn'],
  ['error', 'error'],
];

const FN2 = [
  ['pow', 'pow'],
  ['atan2', 'atan2'],
  ['hypot', 'hypot'],
  ['fmod', 'fmod'],
  ['remainder', 'remainder'],
  ['copysign', 'copysign'],
  ['log_at', 'log_at'],
];

export const ORDER = {
  ATOMIC: 0,
  UNARY: 1,
  MUL: 2,
  ADD: 3,
  COMPARE: 4,
  AND: 5,
  OR: 6,
  WALRUS: 7,
  NONE: 99,
} as const;

const OP_ORDER: Record<string, number> = {
  '*': ORDER.MUL,
  '/': ORDER.MUL,
  '%': ORDER.MUL,
  '+': ORDER.ADD,
  '-': ORDER.ADD,
  '<': ORDER.COMPARE,
  '<=': ORDER.COMPARE,
  '>': ORDER.COMPARE,
  '>=': ORDER.COMPARE,
  '==': ORDER.COMPARE,
  '!=': ORDER.COMPARE,
  is: ORDER.COMPARE,
  and: ORDER.AND,
  or: ORDER.OR,
};

// ---------- 主题：近黑玻璃 + 细描边，强调色只用于选中态 ----------
export let lumeDarkTheme: Blockly.Theme;
export let lumeLightTheme: Blockly.Theme;
let themesDefined = false;

export function defineLumeThemes(): void {
  if (themesDefined) return;
  themesDefined = true;

  const categoryStyles = {
    function_blocks: { colour: '#33aaff' },
    variable_blocks: { colour: '#34d399' },
    control_blocks: { colour: '#a78bfa' },
    value_blocks: { colour: '#fbbf24' },
    call_blocks: { colour: '#f472b6' },
    comment_blocks: { colour: '#8a8a90' },
  };
  const common = {
    blockStyles: {
      ...({} as Record<string, Partial<Blockly.Theme.BlockStyle>>),
      function_blocks: {
        colourPrimary: '#162332',
        colourSecondary: '#101a26',
        colourTertiary: '#0b131c',
        hat: 'cap',
      },
      control_blocks: {
        colourPrimary: '#1c1b2d',
        colourSecondary: '#151424',
        colourTertiary: '#0f0e1a',
      },
      variable_blocks: {
        colourPrimary: '#162b26',
        colourSecondary: '#10221e',
        colourTertiary: '#0b1815',
      },
      value_blocks: {
        colourPrimary: '#2b2418',
        colourSecondary: '#221d14',
        colourTertiary: '#18140f',
      },
      call_blocks: {
        colourPrimary: '#2a1d28',
        colourSecondary: '#211720',
        colourTertiary: '#171017',
      },
      comment_blocks: {
        colourPrimary: '#202125',
        colourSecondary: '#17181b',
        colourTertiary: '#101114',
      },
    },
    categoryStyles,
    fontStyle: {
      family: 'Inter, "Noto Sans CJK SC", sans-serif',
      weight: '500',
      size: 12,
    },
  };

  lumeDarkTheme = Blockly.Theme.defineTheme('lume-dark', {
    ...common,
    base: Blockly.Themes.Classic,
    componentStyles: {
      workspaceBackgroundColour: '#0b0b0e',
      toolboxBackgroundColour: '#0b0b0e',
      toolboxForegroundColour: '#d8d8de',
      flyoutBackgroundColour: '#0e0e12',
      flyoutForegroundColour: '#d8d8de',
      flyoutOpacity: 0.98,
      scrollbarColour: '#3a3a43',
      scrollbarOpacity: 0.45,
      insertionMarkerColour: '#33aaff',
      insertionMarkerOpacity: 0.35,
      cursorColour: '#33aaff',
      selectedGlowColour: '#33aaff',
      selectedGlowOpacity: 0.55,
      replacementGlowColour: '#33aaff',
      replacementGlowOpacity: 0.55,
    },
  } as unknown as Blockly.Theme);

  lumeLightTheme = Blockly.Theme.defineTheme('lume-light', {
    ...common,
    base: Blockly.Themes.Classic,
    componentStyles: {
      workspaceBackgroundColour: '#f7f7f6',
      toolboxBackgroundColour: '#f7f7f6',
      toolboxForegroundColour: '#26262b',
      flyoutBackgroundColour: '#ffffff',
      flyoutForegroundColour: '#26262b',
      flyoutOpacity: 1,
      scrollbarColour: '#a9a9b0',
      scrollbarOpacity: 0.5,
      insertionMarkerColour: '#0077cc',
      insertionMarkerOpacity: 0.35,
      cursorColour: '#0077cc',
      selectedGlowColour: '#0077cc',
      selectedGlowOpacity: 0.5,
      replacementGlowColour: '#0077cc',
      replacementGlowOpacity: 0.5,
    },
  } as unknown as Blockly.Theme);
}

// ---------- 积木定义 ----------
let defined = false;

export function defineLumeBlocks(): void {
  if (defined) return;
  defined = true;

  Blockly.defineBlocksWithJsonArray([
    {
      type: 'lume_function',
      message0: '函数 %1 名称 %2 参数 %3 返回 %4 体 %5',
      args0: [
        { type: 'field_label', name: 'LABEL', text: 'FN' },
        { type: 'field_input', name: 'NAME', text: 'main' },
        { type: 'field_input', name: 'PARAMS', text: '' },
        {
          type: 'field_dropdown',
          name: 'RET',
          options: [
            ['int', 'int'],
            ['float', 'float'],
            ['bool', 'bool'],
            ['string', 'string'],
            ['无返回值', ''],
          ],
        },
        { type: 'input_statement', name: 'BODY' },
      ],
      style: 'function_blocks',
      tooltip: '定义函数。ret 会直接结束函数，末尾无需 end。',
      helpUrl: '',
    },
    {
      type: 'lume_declare',
      message0: '声明 %1 : %2 = %3',
      args0: [
        { type: 'field_input', name: 'NAME', text: 'value' },
        { type: 'field_dropdown', name: 'TYPE', options: TYPES },
        { type: 'input_value', name: 'VALUE', check: 'LumeValue' },
      ],
      previousStatement: null,
      nextStatement: null,
      style: 'variable_blocks',
      tooltip: 'x:int = 10',
      helpUrl: '',
    },
    {
      type: 'lume_assign',
      message0: '赋值 %1 = %2',
      args0: [
        { type: 'field_input', name: 'NAME', text: 'value' },
        { type: 'input_value', name: 'VALUE', check: 'LumeValue' },
      ],
      previousStatement: null,
      nextStatement: null,
      style: 'variable_blocks',
      tooltip: '普通赋值是语句，要求变量已声明。',
      helpUrl: '',
    },
    {
      type: 'lume_walrus_stmt',
      message0: '海象赋值 %1 := %2',
      args0: [
        { type: 'field_input', name: 'NAME', text: 'value' },
        { type: 'input_value', name: 'VALUE', check: 'LumeValue' },
      ],
      previousStatement: null,
      nextStatement: null,
      style: 'variable_blocks',
      tooltip: 'x := 200，更新已有变量并返回赋入的值。',
      helpUrl: '',
    },
    {
      type: 'lume_if',
      message0: '如果 %1 则 %2 否则 %3',
      args0: [
        { type: 'input_value', name: 'COND', check: 'LumeValue' },
        { type: 'input_statement', name: 'DO' },
        { type: 'input_statement', name: 'ELSE' },
      ],
      previousStatement: null,
      nextStatement: null,
      style: 'control_blocks',
      tooltip: 'if / else 代码块。全部分支以 ret 结束时无需 end。',
      helpUrl: '',
    },
    {
      type: 'lume_if_expr',
      message0: '块表达式 若 %1 则 %2 否则 %3',
      args0: [
        { type: 'input_value', name: 'COND', check: 'LumeValue' },
        { type: 'input_statement', name: 'DO' },
        { type: 'input_statement', name: 'ELSE' },
      ],
      output: 'LumeValue',
      style: 'control_blocks',
      tooltip: 'res:int = if cond ... else ... end；块本身产出一个值。',
      helpUrl: '',
    },
    {
      type: 'lume_while',
      message0: '当 %1 循环 %2',
      args0: [
        { type: 'input_value', name: 'COND', check: 'LumeValue' },
        { type: 'input_statement', name: 'DO' },
      ],
      previousStatement: null,
      nextStatement: null,
      style: 'control_blocks',
      tooltip: 'while 条件 ... end',
      helpUrl: '',
    },
    {
      type: 'lume_ret',
      message0: '返回 %1',
      args0: [{ type: 'input_value', name: 'VALUE', check: 'LumeValue' }],
      previousStatement: null,
      style: 'control_blocks',
      tooltip: 'ret 直接结束整个函数。',
      helpUrl: '',
    },
    {
      type: 'lume_print',
      message0: '打印 %1',
      args0: [{ type: 'input_value', name: 'VALUE', check: 'LumeValue' }],
      previousStatement: null,
      nextStatement: null,
      style: 'call_blocks',
      tooltip: 'print(value)',
      helpUrl: '',
    },
    {
      type: 'lume_comment',
      message0: '注释 %1',
      args0: [{ type: 'field_input', name: 'TEXT', text: 'TODO' }],
      previousStatement: null,
      nextStatement: null,
      style: 'comment_blocks',
      tooltip: '# 单行注释',
      helpUrl: '',
    },
    {
      type: 'lume_number',
      message0: '%1',
      args0: [{ type: 'field_number', name: 'NUM', value: 0, precision: 1 }],
      output: 'LumeValue',
      style: 'value_blocks',
      tooltip: '整数或浮点数（浮点数请带小数点）。',
      helpUrl: '',
    },
    {
      type: 'lume_text',
      message0: '文本 %1',
      args0: [{ type: 'field_input', name: 'TEXT', text: 'hello lume' }],
      output: 'LumeValue',
      style: 'value_blocks',
      tooltip: '字符串字面量。',
      helpUrl: '',
    },
    {
      type: 'lume_bool',
      message0: '布尔 %1',
      args0: [
        {
          type: 'field_dropdown',
          name: 'VALUE',
          options: [
            ['true', 'true'],
            ['false', 'false'],
          ],
        },
      ],
      output: 'LumeValue',
      style: 'value_blocks',
      helpUrl: '',
    },
    {
      type: 'lume_var',
      message0: '变量 %1',
      args0: [{ type: 'field_input', name: 'NAME', text: 'value' }],
      output: 'LumeValue',
      style: 'variable_blocks',
      tooltip: '读取局部变量。',
      helpUrl: '',
    },
    {
      type: 'lume_binary',
      message0: '%1 %2 %3',
      args0: [
        { type: 'input_value', name: 'A', check: 'LumeValue' },
        {
          type: 'field_dropdown',
          name: 'OP',
          options: [
            ['+', '+'],
            ['−', '-'],
            ['×', '*'],
            ['÷', '/'],
            ['%', '%'],
            ['==', '=='],
            ['!=', '!='],
            ['<', '<'],
            ['<=', '<='],
            ['>', '>'],
            ['>=', '>='],
            ['is', 'is'],
            ['and', 'and'],
            ['or', 'or'],
          ],
        },
        { type: 'input_value', name: 'B', check: 'LumeValue' },
      ],
      output: 'LumeValue',
      style: 'value_blocks',
      helpUrl: '',
    },
    {
      type: 'lume_unary',
      message0: '%1 %2',
      args0: [
        {
          type: 'field_dropdown',
          name: 'OP',
          options: [
            ['not', 'not'],
            ['−', '-'],
          ],
        },
        { type: 'input_value', name: 'A', check: 'LumeValue' },
      ],
      output: 'LumeValue',
      style: 'value_blocks',
      helpUrl: '',
    },
    {
      type: 'lume_walrus_expr',
      message0: '海象表达式 %1 := %2',
      args0: [
        { type: 'field_input', name: 'NAME', text: 'value' },
        { type: 'input_value', name: 'VALUE', check: 'LumeValue' },
      ],
      output: 'LumeValue',
      style: 'variable_blocks',
      tooltip: 'x := expr，可作为表达式出现在 if / while 条件中。',
      helpUrl: '',
    },
    {
      type: 'lume_call1',
      message0: '%1 ( %2 )',
      args0: [
        { type: 'field_dropdown', name: 'FN', options: FN1 },
        { type: 'input_value', name: 'ARG', check: 'LumeValue' },
      ],
      output: 'LumeValue',
      style: 'call_blocks',
      tooltip: '单参数内置函数 / 命名函数调用。',
      helpUrl: '',
    },
    {
      type: 'lume_call2',
      message0: '%1 ( %2 , %3 )',
      args0: [
        { type: 'field_dropdown', name: 'FN', options: FN2 },
        { type: 'input_value', name: 'A', check: 'LumeValue' },
        { type: 'input_value', name: 'B', check: 'LumeValue' },
      ],
      output: 'LumeValue',
      style: 'call_blocks',
      tooltip: '双参数内置函数，例如 pow(x, y)。',
      helpUrl: '',
    },
    {
      type: 'lume_array',
      message0: '数组 [ %1 %2 %3 %4 ]',
      args0: [
        { type: 'input_value', name: 'ITEM0', check: 'LumeValue' },
        { type: 'input_value', name: 'ITEM1', check: 'LumeValue' },
        { type: 'input_value', name: 'ITEM2', check: 'LumeValue' },
        { type: 'input_value', name: 'ITEM3', check: 'LumeValue' },
      ],
      output: 'LumeValue',
      style: 'value_blocks',
      tooltip: '数组字面量，最多四个元素。',
      helpUrl: '',
    },
    {
      type: 'lume_index',
      message0: '取元素 %1 [ %2 ]',
      args0: [
        { type: 'input_value', name: 'BASE', check: 'LumeValue' },
        { type: 'input_value', name: 'INDEX', check: 'LumeValue' },
      ],
      output: 'LumeValue',
      style: 'value_blocks',
      helpUrl: '',
    },
  ]);
}

// ---------- 工具箱 ----------
export const toolbox = {
  kind: 'categoryToolbox',
  contents: [
    {
      kind: 'category',
      name: '结构',
      categorystyle: 'function_blocks',
      contents: [
        { kind: 'block', type: 'lume_function' },
        { kind: 'block', type: 'lume_comment' },
      ],
    },
    {
      kind: 'category',
      name: '变量',
      categorystyle: 'variable_blocks',
      contents: [
        { kind: 'block', type: 'lume_declare' },
        { kind: 'block', type: 'lume_assign' },
        { kind: 'block', type: 'lume_walrus_stmt' },
        { kind: 'block', type: 'lume_var' },
        { kind: 'block', type: 'lume_walrus_expr' },
      ],
    },
    {
      kind: 'category',
      name: '控制',
      categorystyle: 'control_blocks',
      contents: [
        { kind: 'block', type: 'lume_if' },
        { kind: 'block', type: 'lume_while' },
        { kind: 'block', type: 'lume_if_expr' },
        { kind: 'block', type: 'lume_ret' },
      ],
    },
    {
      kind: 'category',
      name: '运算',
      categorystyle: 'value_blocks',
      contents: [
        { kind: 'block', type: 'lume_number' },
        { kind: 'block', type: 'lume_text' },
        { kind: 'block', type: 'lume_bool' },
        { kind: 'block', type: 'lume_binary' },
        { kind: 'block', type: 'lume_unary' },
        { kind: 'block', type: 'lume_array' },
        { kind: 'block', type: 'lume_index' },
      ],
    },
    {
      kind: 'category',
      name: '调用',
      categorystyle: 'call_blocks',
      contents: [
        { kind: 'block', type: 'lume_print' },
        { kind: 'block', type: 'lume_call1' },
        { kind: 'block', type: 'lume_call2' },
      ],
    },
  ],
};

export const initialWorkspaceXml = `
<xml xmlns="https://developers.google.com/blockly/xml">
  <block type="lume_function" x="42" y="36">
    <field name="NAME">main</field>
    <field name="PARAMS"></field>
    <field name="RET">int</field>
    <statement name="BODY">
      <block type="lume_comment">
        <field name="TEXT">1 到 5 的求和</field>
        <next>
          <block type="lume_declare">
            <field name="NAME">total</field>
            <field name="TYPE">int</field>
            <value name="VALUE">
              <block type="lume_number"><field name="NUM">0</field></block>
            </value>
            <next>
              <block type="lume_declare">
                <field name="NAME">i</field>
                <field name="TYPE">int</field>
                <value name="VALUE">
                  <block type="lume_number"><field name="NUM">1</field></block>
                </value>
                <next>
                  <block type="lume_while">
                    <value name="COND">
                      <block type="lume_binary">
                        <field name="OP">&lt;=</field>
                        <value name="A">
                          <block type="lume_var"><field name="NAME">i</field></block>
                        </value>
                        <value name="B">
                          <block type="lume_number"><field name="NUM">5</field></block>
                        </value>
                      </block>
                    </value>
                    <statement name="DO">
                      <block type="lume_walrus_stmt">
                        <field name="NAME">total</field>
                        <value name="VALUE">
                          <block type="lume_binary">
                            <field name="OP">+</field>
                            <value name="A">
                              <block type="lume_var"><field name="NAME">total</field></block>
                            </value>
                            <value name="B">
                              <block type="lume_var"><field name="NAME">i</field></block>
                            </value>
                          </block>
                        </value>
                        <next>
                          <block type="lume_walrus_stmt">
                            <field name="NAME">i</field>
                            <value name="VALUE">
                              <block type="lume_binary">
                                <field name="OP">+</field>
                                <value name="A">
                                  <block type="lume_var"><field name="NAME">i</field></block>
                                </value>
                                <value name="B">
                                  <block type="lume_number"><field name="NUM">1</field></block>
                                </value>
                              </block>
                            </value>
                          </block>
                        </next>
                      </block>
                    </statement>
                    <next>
                      <block type="lume_print">
                        <value name="VALUE">
                          <block type="lume_var"><field name="NAME">total</field></block>
                        </value>
                        <next>
                          <block type="lume_ret">
                            <value name="VALUE">
                              <block type="lume_var"><field name="NAME">total</field></block>
                            </value>
                          </block>
                        </next>
                      </block>
                    </next>
                  </block>
                </next>
              </block>
            </next>
          </block>
        </next>
      </block>
    </statement>
  </block>
</xml>
`;

// ---------- 生成器 ----------
export const lumeGenerator = new Blockly.Generator('Lume');
lumeGenerator.INDENT = '    ';

type Block = Blockly.Block;
type ValueResult = [string, number];

function value(g: Blockly.Generator, block: Block, input: string, order: number, fallback = '0'): string {
  return g.valueToCode(block, input, order) || fallback;
}

function num(field: string | number | null): string {
  const raw = String(field ?? '0').trim();
  return raw || '0';
}

function stringLiteral(text: string): string {
  return JSON.stringify(text ?? '');
}

function topStatements(code: string): string[] {
  const lines = code.split('\n').filter((line) => line.trim().length > 0);
  if (!lines.length) return [];
  const indent = (line: string) => /^[\t ]*/.exec(line)?.[0].length ?? 0;
  const min = Math.min(...lines.map(indent));
  return lines.filter((line) => indent(line) === min).map((line) => line.trim());
}

function terminatedByRet(code: string): boolean {
  const stmts = topStatements(code);
  return stmts.length > 0 && /^ret\b/.test(stmts[stmts.length - 1]);
}

function blockExpressionEnd(code: string): string {
  return code.replace(/(\r?\n)?\s*end\s*$/, '');
}

lumeGenerator.forBlock['lume_function'] = (block, generator) => {
  const name = String(block.getFieldValue('NAME') || 'main').trim() || 'main';
  const params = String(block.getFieldValue('PARAMS') || '').trim();
  const ret = String(block.getFieldValue('RET') || '');
  const body = generator.statementToCode(block, 'BODY');
  const signature = `fn ${name}(${params})${ret ? ` -> ${ret}` : ''}`;
  return `${signature}\n${body}${terminatedByRet(body) ? '' : 'end\n'}`;
};

lumeGenerator.forBlock['lume_declare'] = (block, generator) => {
  const name = String(block.getFieldValue('NAME') || 'value').trim() || 'value';
  const type = String(block.getFieldValue('TYPE') || 'int');
  const init = value(generator, block, 'VALUE', ORDER.NONE);
  return `${name}:${type} = ${init}\n`;
};

lumeGenerator.forBlock['lume_assign'] = (block, generator) => {
  const name = String(block.getFieldValue('NAME') || 'value').trim() || 'value';
  const expr = value(generator, block, 'VALUE', ORDER.NONE);
  return `${name} = ${expr}\n`;
};

lumeGenerator.forBlock['lume_walrus_stmt'] = (block, generator) => {
  const name = String(block.getFieldValue('NAME') || 'value').trim() || 'value';
  const expr = value(generator, block, 'VALUE', ORDER.NONE);
  return `${name} := ${expr}\n`;
};

lumeGenerator.forBlock['lume_if'] = (block, generator) => {
  const cond = value(generator, block, 'COND', ORDER.NONE, 'false');
  const thenCode = generator.statementToCode(block, 'DO');
  const elseCode = generator.statementToCode(block, 'ELSE');
  let code = `if ${cond}\n${thenCode}`;
  if (elseCode.trim()) {
    code += `else\n${elseCode}`;
  }
  const allReturn = terminatedByRet(thenCode) && (!elseCode.trim() || terminatedByRet(elseCode));
  return `${code}${allReturn ? '' : 'end\n'}`;
};

lumeGenerator.forBlock['lume_if_expr'] = (block, generator): ValueResult => {
  const cond = value(generator, block, 'COND', ORDER.NONE, 'false');
  const thenCode = blockExpressionEnd(generator.statementToCode(block, 'DO'));
  const elseCode = blockExpressionEnd(generator.statementToCode(block, 'ELSE'));
  let code = `if ${cond}\n${thenCode}`;
  if (elseCode.trim()) code += `\nelse\n${elseCode}`;
  code += '\nend';
  return [code, ORDER.ATOMIC];
};

lumeGenerator.forBlock['lume_while'] = (block, generator) => {
  const cond = value(generator, block, 'COND', ORDER.NONE, 'false');
  const body = generator.statementToCode(block, 'DO');
  return `while ${cond}\n${body}end\n`;
};

lumeGenerator.forBlock['lume_ret'] = (block, generator) => {
  const expr = generator.valueToCode(block, 'VALUE', ORDER.NONE);
  return expr ? `ret ${expr}\n` : 'ret\n';
};

lumeGenerator.forBlock['lume_print'] = (block, generator) => {
  const expr = value(generator, block, 'VALUE', ORDER.NONE);
  return `print(${expr})\n`;
};

lumeGenerator.forBlock['lume_comment'] = (block) => {
  const text = String(block.getFieldValue('TEXT') || '').replace(/\r?\n/g, ' ');
  return `# ${text}\n`;
};

lumeGenerator.forBlock['lume_number'] = (block): ValueResult => [num(block.getFieldValue('NUM')), ORDER.ATOMIC];

lumeGenerator.forBlock['lume_text'] = (block): ValueResult => [
  stringLiteral(String(block.getFieldValue('TEXT') ?? '')),
  ORDER.ATOMIC,
];

lumeGenerator.forBlock['lume_bool'] = (block): ValueResult => [
  block.getFieldValue('VALUE') === 'false' ? 'false' : 'true',
  ORDER.ATOMIC,
];

lumeGenerator.forBlock['lume_var'] = (block): ValueResult => [
  String(block.getFieldValue('NAME') || 'value').trim() || 'value',
  ORDER.ATOMIC,
];

lumeGenerator.forBlock['lume_binary'] = (block, generator): ValueResult => {
  const op = String(block.getFieldValue('OP') || '+');
  const order = OP_ORDER[op] ?? ORDER.ATOMIC;
  const lhs = value(generator, block, 'A', order);
  const rhs = value(generator, block, 'B', order);
  return [`${lhs} ${op} ${rhs}`, order];
};

lumeGenerator.forBlock['lume_unary'] = (block, generator): ValueResult => {
  const op = String(block.getFieldValue('OP') || 'not');
  const inner = value(generator, block, 'A', ORDER.UNARY);
  return [op === '-' ? `-${inner}` : `not ${inner}`, ORDER.UNARY];
};

lumeGenerator.forBlock['lume_walrus_expr'] = (block, generator): ValueResult => {
  const name = String(block.getFieldValue('NAME') || 'value').trim() || 'value';
  const expr = value(generator, block, 'VALUE', ORDER.WALRUS);
  return [`${name} := ${expr}`, ORDER.WALRUS];
};

lumeGenerator.forBlock['lume_call1'] = (block, generator): ValueResult => {
  const fn = String(block.getFieldValue('FN') || 'print');
  const arg = value(generator, block, 'ARG', ORDER.NONE);
  return [`${fn}(${arg})`, ORDER.ATOMIC];
};

lumeGenerator.forBlock['lume_call2'] = (block, generator): ValueResult => {
  const fn = String(block.getFieldValue('FN') || 'pow');
  const a = value(generator, block, 'A', ORDER.NONE);
  const b = value(generator, block, 'B', ORDER.NONE);
  return [`${fn}(${a}, ${b})`, ORDER.ATOMIC];
};

lumeGenerator.forBlock['lume_array'] = (block, generator): ValueResult => {
  const items = ['ITEM0', 'ITEM1', 'ITEM2', 'ITEM3']
    .map((input) => generator.valueToCode(block, input, ORDER.NONE))
    .filter((item) => item.trim().length > 0);
  return [`[${items.join(', ')}]`, ORDER.ATOMIC];
};

lumeGenerator.forBlock['lume_index'] = (block, generator): ValueResult => {
  const base = value(generator, block, 'BASE', ORDER.ATOMIC);
  const index = value(generator, block, 'INDEX', ORDER.NONE);
  return [`${base}[${index}]`, ORDER.ATOMIC];
};

export function generateLume(workspace: Blockly.Workspace): string {
  const body = lumeGenerator.workspaceToCode(workspace).trim();
  return `# Generated by Lume Studio · 积木 → Lume 源码\n${body ? `${body}\n` : ''}`;
}

export function loadExample(workspace: Blockly.Workspace): void {
  workspace.clear();
  Blockly.Xml.domToWorkspace(Blockly.utils.xml.textToDom(initialWorkspaceXml), workspace);
}
