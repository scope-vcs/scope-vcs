#!/usr/bin/env python3
"""Derive disposable staging infrastructure from the actual Fargate configuration."""
import json
import pathlib
import sys
import yaml


class CfnLoader(yaml.SafeLoader):
    pass


def intrinsic(loader, tag, node):
    if isinstance(node, yaml.ScalarNode):
        value = loader.construct_scalar(node)
    else:
        value = loader.construct_sequence(node)
    if tag == "GetAtt" and isinstance(value, str):
        value = value.split(".")
    return {tag if tag == "Ref" else "Fn::" + tag: value}


CfnLoader.add_multi_constructor("!", intrinsic)
root = pathlib.Path(__file__).resolve().parents[2]
template = yaml.load((root / "deploy/aws/cloud-runner.yaml").read_text(), Loader=CfnLoader)
resources = [
    "RunnerVpc", "RunnerInternetGateway", "RunnerInternetGatewayAttachment",
    "RunnerPublicRouteTable", "RunnerPublicRoute", "RunnerPublicSubnetOne",
    "RunnerPublicSubnetTwo", "RunnerPublicSubnetOneRouteTableAssociation",
    "RunnerPublicSubnetTwoRouteTableAssociation", "RunnerSecurityGroup",
    "RunnerCluster", "RunnerLogGroup", "RunnerTaskExecutionRole",
    "RailwayDispatcherUser", "RailwayDispatcherPolicy",
]
template["Resources"] = {key: template["Resources"][key] for key in resources}
template["Parameters"] = {key: template["Parameters"][key] for key in [
    "Environment", "VpcCidr", "PublicSubnetOneCidr", "PublicSubnetTwoCidr",
]}
template["Parameters"]["Environment"] = {
    "Type": "String", "Default": "cache-db-staging-20260907",
    "AllowedValues": ["cache-db-staging-20260907"],
}
template.pop("Conditions")
template["Description"] = "Disposable staging cache and database experiment, using production Fargate networking"
for policy in template["Resources"]["RunnerTaskExecutionRole"]["Properties"]["Policies"]:
    statements = policy["PolicyDocument"]["Statement"]
    statements[:] = [statement for statement in statements if "Fn::If" not in statement]
    for statement in statements:
        if statement.get("Resource") == {"Fn::GetAtt": ["ChecksImageRepository", "Arn"]}:
            statement["Resource"] = {"Fn::Sub": "arn:${AWS::Partition}:ecr:${AWS::Region}:${AWS::AccountId}:repository/scope-vcs/production/checks"}
template["Outputs"] = {key: value for key, value in template["Outputs"].items() if key in [
    "AwsRegion", "RunnerClusterArn", "RunnerSubnetIds", "RunnerSecurityGroupId",
    "RunnerExecutionRoleArn", "RunnerLogGroupName", "RailwayDispatcherUserName",
]}
pathlib.Path(sys.argv[1]).write_text(json.dumps(template, indent=2) + "\n")
