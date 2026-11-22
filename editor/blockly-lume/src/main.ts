import * as Blockly from 'blockly';

// ---------------- Lume 简易生成器，只实现几个积木示例 ----------------
const lumeGenerator = new Blockly.Generator('Lume');

// 变量赋值
lumeGenerator.forBlock['variables_set'] = function(block: Blockly.Block) {
  const varName = lumeGenerator.getVariableName(block.getFieldValue('VAR'));
  const valueCode = lumeGenerator.valueToCode(block, 'VALUE', Blockly.Order.ASSIGNMENT) || '0';
  return `${varName} = ${valueCode};\n`;
};

// print 输出
lumeGenerator.forBlock['lume_print'] = function(block: Blockly.Block) {
  const arg = lumeGenerator.valueToCode(block, 'ARG', Blockly.Order.NONE) || '';
  return `print(${arg});\n`;
};

// if 条件
lumeGenerator.forBlock['controls_if'] = function(block: Blockly.Block) {
  let code = '';
  for(let i=0; i < block.getTimesRepeated(); i++){
    const cond = lumeGenerator.valueToCode(block, `IF${i}`, Blockly.Order.COMPARISON) || 'false';
    const branch = lumeGenerator.statementToCode(block, `DO${i}`);
    if(i === 0){
      code += `if(${cond}) {\n${branch}}\n`;
    }else{
      code += `else if(${cond}) {\n${branch}}\n`;
    }
  }
  if(block.getInput('ELSE')){
    const elseCode = lumeGenerator.statementToCode(block, 'ELSE');
    code += `else {\n${elseCode}}\n`;
  }
  return code;
};

// ---------------- 工具箱定义 ----------------
const toolboxXml = `
<xml xmlns="https://developers.google.com/blockly/xml">
  <category name="变量" colour="#a55b45">
    <block type="variables_set">
      <field name="VAR">a</field>
      <value name="VALUE">
        <block type="math_number"><field name="NUM">10</field></block>
      </value>
    </block>
  </category>
  <category name="控制" colour="#5b80a5">
    <block type="controls_if"></block>
  </category>
  <category name="输出" colour="#5ba55b">
    <block type="lume_print">
      <value name="ARG">
        <block type="text"><field name="TEXT">hello lume</field></block>
      </value>
    </block>
  </category>
  <category name="数学" colour="#5b67a5">
    <block type="math_number"></block>
    <block type="math_arithmetic"></block>
  </category>
  <category name="文本" colour="#a5745b">
    <block type="text"></block>
  </category>
</xml>
`;

// 初始化工作区
const blocklyDiv = document.getElementById('blocklyDiv')!;
const codeOutput = document.getElementById('codeOutput') as HTMLTextAreaElement;

const workspace = Blockly.inject(blocklyDiv, {
  toolbox: toolboxXml,
  scrollbars: true,
  zoom: {
    controls: true,
    wheel: true,
    startScale: 1.0,
    maxScale: 3,
    minScale: 0.3,
    scaleSpeed: 1.2
  }
});

// 画布变化 → 生成Lume代码，输出到文本框
workspace.addChangeListener(() => {
  const code = lumeGenerator.workspaceToCode(workspace);
  codeOutput.value = code;
});

// 可选：预放一个积木在画布
Blockly.Xml.domToWorkspace(Blockly.utils.xml.textToDom(toolboxXml), workspace);
